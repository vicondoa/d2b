//! The virtiofsd Volume chain check, ported from its fixture.
//!
//! It is the U11 midpoint Volume proof for the v3 resource plane, separate
//! from the Process-slice operator-activation check: the Nix-ingested Volume
//! derives one deterministic VolumeBinding child through the manager, that
//! binding owns the virtiofsd worker Process and its private Endpoint, the
//! worker is realized and binds its private serving socket, and deleting the
//! Volume tears the chain down. Every wait asserts on observable resource
//! identity, phase, generation, processes and sockets - read through the
//! installed d2b CLI and the host filesystem - and nothing here keys on
//! informational log text.
//!
//! The assertions are the fixture's, in the fixture's order and with the
//! fixture's own command text and bounds, and so are the row dumps its
//! failures print: the fixture's own `chain_row_dumps`, which every
//! volume-chain wait carries, and its `dump_rows`, which the teardown
//! window's failure path runs before it reports. The two identities the
//! fixture derives - the binding name from the attachment tuple and the
//! serving socket path from (zone, volume, guest) - are its own frozen-v1
//! derivations, kept here as the constants they resolved to.
//!
//! The teardown window is sampled the way the fixture sampled it: the three
//! lists are read parent -> worker -> endpoint, so a parent observed gone
//! ahead of a child means the child was already gone when the parent retired
//! rather than that the two reads straddled the teardown, and the samples are
//! asserted over afterwards for endpoint-first ordering and for no owned row
//! outliving its parent.
//!
//! The guest is the reusable daemon node plus the fixture's own contributions
//! (nftables, the acceptance host runtime, the Volume acceptance artifact and
//! its publisher key, the two zones and their rows, the two declared users and
//! the operator role binding), declared in
//! `nix/test-support/host-integration-node.nix`. `start_all()` is not
//! restated here: it is the lane's own boot of the guest the check runs
//! against.

use std::{thread, time::Duration};

use serde_json::Value;

use crate::legacy::{DiagRow, GuestControl, LegacyError, LegacyResult};

/// The bound the nftables unit's boot wait gets, the fixture's own.
const BOOT: Duration = Duration::from_secs(180);

/// The bound the socket-activated broker socket and the public socket's file
/// wait get, the fixture's own.
const SOCKET_ACTIVATION: Duration = Duration::from_secs(30);

/// The bound the daemon's unit wait gets, the fixture's own.
const DAEMON_UP: Duration = Duration::from_secs(180);

/// The bound the Volume realize wait gets, the fixture's own.
const VOLUME_REALIZED: Duration = Duration::from_secs(180);

/// The bound each derived-child realize wait gets, the fixture's own.
const CHILD_REALIZED: Duration = Duration::from_secs(120);

/// The bound the serving socket and the worker process get, the fixture's
/// own.
const SERVING: Duration = Duration::from_secs(60);

/// The bound the binding's own deleting wait gets, the fixture's own.
const BINDING_DELETING: Duration = Duration::from_secs(60);

/// How many times the teardown window is sampled, the fixture's own bound:
/// six hundred samples with the driver's interval between them.
const TEARDOWN_SAMPLES: usize = 600;

/// The interval between two samples, the fixture's own `time.sleep(0.2)`.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);

/// The daemon journal lines that explain a worker Process whose launch effect
/// never completed.
///
/// The daemon's own tail for this stage is dominated by the unrelated layout
/// retry on the store view, so a reader cannot see the launch refusal in it.
/// The supervisor names the effect error it projected and the broker backend
/// names the leg under it, so these tokens are the record of which leg
/// refused a launch no row states on its own.
const WORKER_LAUNCH_JOURNAL: &[DiagRow<'_>] = &[
    ("d2bd.service", "supervisor launch effect failed"),
    ("d2bd.service", "broker refused a process request"),
    ("d2bd.service", "broker spawn invocation failed"),
    ("d2bd.service", "broker transport failed for a process request"),
    ("d2bd.service", "process provider effect failed"),
    ("d2bd.service", "process launch failed"),
    ("d2bd.service", "launch request rejected"),
    // The relay drops a refusal's detail from the response envelope and logs
    // it here instead, so this is the only line that says WHY the broker
    // refused a spawn.
    ("d2bd.service", "forwarded invocation refused with a reason"),
    ("d2bd.service", "reply timeout"),
    // The adoption leg the driver runs before every launch, and the broker's
    // duplicate-registration guard, are the two lines that say whether a row
    // relaunched a live child or lost one.
    ("d2bd.service", "broker observe invocation failed"),
    ("d2bd.service", "reserve: reclaiming"),
    ("d2bd.service", "broker pidfd reply carried no result"),
];

/// The deterministic VolumeBinding identity the frozen v1 derivation pins,
/// the fixture's own `binding_name`: `vol-binding-` followed by the first
/// twenty-four hex digits of the sha256 of the NUL-joined
/// ("d2b/volume-local/binding/v1", "Volume/state", "Guest/acceptance-guest",
/// "controller", "/state").
const BINDING_NAME: &str = "vol-binding-6a8ea4307a30f7ceae6533f2";

/// The private serving socket the worker binds, the fixture's own
/// derivation: `/run/d2b/vms/acceptance-guest/vol-<tag>.vfd.sock`, where the
/// tag is the first eight hex digits of the sha256 of the NUL-joined
/// ("work", "state", "acceptance-guest").
const SOCKET_PATH: &str = "/run/d2b/vms/acceptance-guest/vol-a424ba7a.vfd.sock";

/// The uuid shape the read path must serve. A uid is not just non-null: the
/// daemon's `ResourceUid` Display is a redaction placeholder, and a
/// placeholder served through the read path would satisfy a null check while
/// breaking every consumer that parses the field.
const UUID_V4: &str = "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";

/// The row projection every volume-chain wait's diagnostics print, the
/// fixture's own `chain_projection`.
const CHAIN_PROJECTION: &str = concat!(
    "[.resources[] | {type: .type, name: .metadata.name, ",
    "owner: .metadata.ownerRef, phase: .status.phase, ",
    "gen: .metadata.generation, obs: .status.observedGeneration, ",
    "template: .spec.template, purpose: .spec.purpose, ",
    "producer: .spec.producerRef}]",
);

/// At least one live virtiofsd process, counted by the fixture's own `ps`
/// pipeline.
const VIRTIOFSD_PROCESS: &str = concat!(
    "test \"$(ps -eo args= | awk '/virtiofsd/ && !/awk/ {c++} END {print c+0}')\" ",
    "-ge 1",
);

/// The revision the delete resolves its exact precondition from, read off the
/// pre-delete list the fixture saved.
const VOLUME_REVISION: &str = concat!(
    "jq -er '.resources[] | select(.type == \"Volume\" and ",
    ".metadata.name == \"state\") | .metadata.revision' ",
    "/run/d2b-volume-pre-delete.json",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    let dumps = chain_row_dumps();
    let rows = rows(&dumps);

    control.stage("boot");
    control.wait_for_unit("nftables.service", None, BOOT)?;
    control.wait_for_unit("d2b-broker.socket", None, SOCKET_ACTIVATION)?;
    control.diag_unit("daemon-up", "d2bd.service", DAEMON_UP)?;
    control.wait_for_file("/run/d2b/public.sock", SOCKET_ACTIVATION)?;

    // 1. Volume realize: the Nix-ingested Volume is Ready with a manager
    //    identity and an observed generation (U10 ingestion -> U7 driver).
    control.diag_wait(
        "volume-realized",
        &volume_realized(),
        VOLUME_REALIZED,
        &rows,
        &[("d2bd.service", "Volume/state")],
    )?;

    // 2. The Volume side minted exactly one deterministic VolumeBinding
    //    child through the manager, and that child converged.
    control.diag_wait(
        "binding-realized",
        &binding_realized(),
        CHILD_REALIZED,
        &rows,
        &[("d2bd.service", BINDING_NAME)],
    )?;

    // 3. The binding owns the virtiofsd worker Process and its private
    //    Endpoint as managed children; the worker is realized (a live
    //    virtiofsd process) and the serving socket is bound.
    control.diag_wait(
        "worker-realized",
        &worker_realized(),
        CHILD_REALIZED,
        &rows,
        &[("d2bd.service", "virtiofsd-worker")],
    )?;
    control.diag_wait(
        "endpoint-realized",
        &endpoint_realized(),
        CHILD_REALIZED,
        &rows,
        &WORKER_LAUNCH_JOURNAL
            .iter()
            .copied()
            .chain([
                ("d2bd.service", ""),
                ("d2bd.service", "virtiofsd"),
                ("d2b-broker.service", ""),
            ])
            .collect::<Vec<_>>(),
    )?;
    control.diag_wait(
        "serving-socket",
        &format!("test -S {SOCKET_PATH}"),
        SERVING,
        &rows,
        &[("d2bd.service", "virtiofsd")],
    )?;
    control.diag_wait(
        "virtiofsd-process",
        VIRTIOFSD_PROCESS,
        SERVING,
        &rows,
        &[("d2bd.service", "virtiofsd")],
    )?;

    // 4. Deleting the owning Volume drives the whole chain through the
    //    preserved endpoint-first teardown: the binding is marked deleting
    //    first, its private Endpoint and socket are removed before the
    //    worker Process row, and nothing owned survives.
    let pre_delete = d2b("list Volume", "/run/d2b-volume-pre-delete.json");
    control.succeed(&[&pre_delete], None)?;
    let volume_revision = control.succeed(&[VOLUME_REVISION], None)?.trim().to_owned();
    let delete = d2b(
        &format!("delete Volume/state --revision {volume_revision}"),
        "/run/d2b-volume-delete.json",
    );
    // The delete writes its envelope to a file, so a refusal is reported
    // with an empty console: the rows and the daemon's own line for the
    // refused store call are the only record of which refusal it was.
    control.diag_run(
        "volume-delete",
        &delete,
        &rows,
        &[
            ("d2bd.service", "manager-backed store call failed"),
            ("d2bd.service", "delete"),
        ],
    )?;

    control.diag_wait(
        "binding-deleting",
        &binding_deleting(),
        BINDING_DELETING,
        &rows,
        &[("d2bd.service", BINDING_NAME)],
    )?;

    // Sample the teardown window: the Endpoint row and the worker Process
    // row must both disappear, the Endpoint never after the worker, and no
    // owned row may outlive its parent. The three lists are separate reads,
    // so they run parent -> worker -> endpoint: a parent observed gone ahead
    // of a child then really means the child was already gone when the
    // parent retired (the invariant under test), while the reverse order
    // could straddle the teardown and report a violation that never held.
    let lists = teardown_lists();
    let sample_command = teardown_sample();
    let mut observed: Vec<Sample> = Vec::new();
    let mut converged = false;
    for _ in 0..TEARDOWN_SAMPLES {
        control.succeed(&[&lists], None)?;
        let sample = read_sample(&control.succeed(&[&sample_command], None)?)?;
        converged = sample.is_torn_down();
        observed.push(sample);
        if converged {
            break;
        }
        sleep_between_samples();
    }
    if !converged {
        dump_rows(control, "teardown did not converge", &dumps);
        let last = match observed.last() {
            Some(sample) => sample.as_json(),
            // The loop samples before it can leave, so this arm is a guard
            // rather than a state; it is reported rather than panicked on.
            None => "no sample was taken".to_owned(),
        };
        return Err(LegacyError::Assertion(format!(
            "volume teardown did not converge within its budget: {last}"
        )));
    }

    for sample in &observed {
        if sample.binding == 0 && (sample.endpoint != 0 || sample.worker != 0) {
            return Err(LegacyError::Assertion(format!(
                "owned child outlived its parent binding: {}",
                sample.as_python_dict()
            )));
        }
        if sample.worker == 0 && sample.endpoint != 0 {
            return Err(LegacyError::Assertion(format!(
                concat!(
                    "worker Process row disappeared before the binding-owned ",
                    "Endpoint row (endpoint-first teardown violated): {}",
                ),
                sample.as_python_dict()
            )));
        }
    }

    control.succeed(&[&volume_after_delete()], None)?;
    Ok(())
}

/// The CLI read the fixture's own `d2b(command, out)` helper built: one zone,
/// read as `alice` through the public socket, into the file the wait's jq
/// reads.
fn d2b(command: &str, out: &str) -> String {
    format!(
        concat!(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
            "d2b --zone work --json {command} >{out}",
        ),
        command = command,
        out = out,
    )
}

/// The Volume realize wait, the fixture's own command text.
fn volume_realized() -> String {
    format!(
        concat!(
            "{list_volume} && ",
            "jq -e '",
            "([.resources[] | select(.type == \"Volume\" and ",
            ".metadata.name == \"state\")] | length) == 1 and ",
            "(.resources[] | select(.type == \"Volume\" and ",
            ".metadata.name == \"state\") | ",
            "((.metadata.uid | test(\"{uuid_v4}\")) and ",
            ".metadata.generation > 0 and ",
            ".metadata.ownerRef == null and ",
            ".status.phase == \"Ready\" and ",
            ".status.observedGeneration == .metadata.generation))' ",
            "/run/d2b-volume-realized.json",
        ),
        list_volume = d2b("list Volume", "/run/d2b-volume-realized.json"),
        uuid_v4 = UUID_V4,
    )
}

/// The VolumeBinding realize wait, the fixture's own command text: exactly
/// one child of the Volume, at the deterministic identity, `Ready` and
/// settled, carrying the attachment tuple it was derived from.
///
/// The consumer-side destination is asserted through the presentation
/// `VolumeBindingSpec` actually commits. That spec has no top-level
/// `mountPath`: the destination is the `filesystem` variant of the closed
/// `presentation` vocabulary, which serializes as
/// `{"presentation":"filesystem","destination":"/state"}` (see
/// `d2b-contracts-resource`'s `spec_serializes_and_round_trips_strictly`).
/// Reading a field the contract does not carry is a wait that can never
/// succeed, so the assertion names the committed shape and stays exactly as
/// strict about the destination it checks.
fn binding_realized() -> String {
    format!(
        concat!(
            "{list_binding} && ",
            "jq -e '",
            "([.resources[] | select(.type == \"VolumeBinding\" and ",
            ".metadata.name == \"{binding_name}\" and ",
            ".metadata.ownerRef == \"Volume/state\")] | length) == 1 and ",
            "(.resources[] | select(.metadata.name == \"{binding_name}\") | ",
            "((.metadata.uid | test(\"{uuid_v4}\")) and ",
            ".metadata.generation > 0 and ",
            ".status.phase == \"Ready\" and ",
            ".status.observedGeneration == .metadata.generation and ",
            ".spec.volumeRef == \"Volume/state\" and ",
            ".spec.executionRef == \"Guest/acceptance-guest\" and ",
            ".spec.view == \"controller\" and ",
            ".spec.access == \"read-only\" and ",
            ".spec.presentation.presentation == \"filesystem\" and ",
            ".spec.presentation.destination == \"/state\"))' ",
            "/run/d2b-binding-realized.json",
        ),
        list_binding = d2b("list VolumeBinding", "/run/d2b-binding-realized.json"),
        binding_name = BINDING_NAME,
        uuid_v4 = UUID_V4,
    )
}

/// The worker Process realize wait, the fixture's own command text: exactly
/// one worker owned by the binding, realized from the signed virtiofsd-worker
/// template, `Ready` and settled.
fn worker_realized() -> String {
    format!(
        concat!(
            "{list_process} && ",
            "jq -e '",
            "([.resources[] | select(.type == \"Process\" and ",
            ".metadata.ownerRef == \"VolumeBinding/{binding_name}\")] | length) ",
            "== 1 and ",
            "([.resources[] | select(.type == \"Process\" and ",
            ".metadata.ownerRef == \"VolumeBinding/{binding_name}\") | ",
            "(.spec.providerRef == \"Provider/system-minijail\" and ",
            ".spec.executionRef == \"Host/host-system\" and ",
            ".spec.processClass == \"worker\" and ",
            ".spec.template == \"virtiofsd-worker\" and ",
            ".status.phase == \"Ready\" and ",
            ".status.observedGeneration == .metadata.generation)] | length) == 1' ",
            "/run/d2b-worker-realized.json",
        ),
        list_process = d2b("list Process", "/run/d2b-worker-realized.json"),
        binding_name = BINDING_NAME,
    )
}

/// The Endpoint realize wait, the fixture's own command text: exactly one
/// private Endpoint owned by the binding, `Ready` and settled, whose producer
/// resolves back to the binding's own worker Process row.
fn endpoint_realized() -> String {
    format!(
        concat!(
            "{list_endpoint} && ",
            "{list_process} && ",
            "jq -e --slurpfile proc /run/d2b-worker-producer.json '",
            "([.resources[] | select(.type == \"Endpoint\" and ",
            ".metadata.ownerRef == \"VolumeBinding/{binding_name}\")] | length) ",
            "== 1 and ",
            "([$proc[0].resources[] | select(.type == \"Process\" and ",
            ".metadata.ownerRef == \"VolumeBinding/{binding_name}\" and ",
            ".status.phase == \"Ready\" and ",
            ".status.observedGeneration == .metadata.generation)] | length) ",
            "== 1 and ",
            "(.resources[] | select(.type == \"Endpoint\" and ",
            ".metadata.ownerRef == \"VolumeBinding/{binding_name}\") | ",
            "(.status.phase == \"Ready\" and ",
            ".status.observedGeneration == .metadata.generation and ",
            ".spec.transport == \"unix\" and ",
            "(.spec.purpose == \"virtiofsd\" and ",
            "(.spec.producerRef as $producer | ",
            "any($proc[0].resources[]; ",
            ".type == \"Process\" and ",
            "\"\\(.type)/\\(.metadata.name)\" == $producer)))))' ",
            "/run/d2b-endpoint-realized.json",
        ),
        list_endpoint = d2b("list Endpoint", "/run/d2b-endpoint-realized.json"),
        list_process = d2b("list Process", "/run/d2b-worker-producer.json"),
        binding_name = BINDING_NAME,
    )
}

/// The binding-deleting wait, the fixture's own command text: the binding is
/// marked deleting before anything owned it is torn down.
fn binding_deleting() -> String {
    format!(
        concat!(
            "{list_binding} && ",
            "jq -e '",
            "any(.resources[]; .type == \"VolumeBinding\" and ",
            ".metadata.name == \"{binding_name}\" and ",
            ".metadata.deletionRequestedAt != null)' ",
            "/run/d2b-binding-deleting.json",
        ),
        list_binding = d2b("list VolumeBinding", "/run/d2b-binding-deleting.json"),
        binding_name = BINDING_NAME,
    )
}

/// The three teardown-window lists in one command, the fixture's own: read
/// parent -> worker -> endpoint and closed by the driver's own `echo OK`.
fn teardown_lists() -> String {
    [
        d2b("list VolumeBinding", "/run/d2b-teardown-binding.json"),
        d2b("list Process", "/run/d2b-teardown-process.json"),
        d2b("list Endpoint", "/run/d2b-teardown-endpoint.json"),
        "echo OK".to_owned(),
    ]
    .join("; ")
}

/// The sample command, the fixture's own `sample_expr`: whether the serving
/// socket is still bound, and what each of the three lists it just read held
/// for the binding and its owned rows. The doubled braces are the JSON object
/// the fixture's jq program builds.
fn teardown_sample() -> String {
    let binding_owner = format!("VolumeBinding/{BINDING_NAME}");
    format!(
        concat!(
            "socket=false; test -S {socket_path} && socket=true; ",
            "jq -n --argjson socket \"$socket\" ",
            "--slurpfile b /run/d2b-teardown-binding.json ",
            "--slurpfile p /run/d2b-teardown-process.json ",
            "--slurpfile e /run/d2b-teardown-endpoint.json ",
            "'{{",
            "endpoint: ([$e[0].resources[] | ",
            "select(.metadata.ownerRef == \"{binding_owner}\")] | length), ",
            "worker: ([$p[0].resources[] | select(.type == \"Process\" and ",
            ".metadata.ownerRef == \"{binding_owner}\")] | length), ",
            "binding: ([$b[0].resources[] | select(.type == \"VolumeBinding\" ",
            "and .metadata.name == \"{binding_name}\")] | length), ",
            "socket: $socket",
            "}}'",
        ),
        socket_path = SOCKET_PATH,
        binding_owner = binding_owner,
        binding_name = BINDING_NAME,
    )
}

/// The post-delete read, the fixture's own command text: no Volume row named
/// `state` is left.
fn volume_after_delete() -> String {
    format!(
        concat!(
            "{list_volume} && ",
            "jq -e 'all(.resources[]; ",
            "(.type == \"Volume\" and .metadata.name == \"state\") | not)' ",
            "/run/d2b-volume-after-delete.json",
        ),
        list_volume = d2b("list Volume", "/run/d2b-volume-after-delete.json"),
    )
}

/// The row set every volume-chain wait asserts on, the fixture's own
/// `chain_row_dumps`: one labelled projection per resource type the chain
/// waits read, plus the directory the worker's serving socket is minted in.
fn chain_row_dumps() -> Vec<(String, String)> {
    let mut dumps = ["Volume", "VolumeBinding", "Process", "Endpoint"]
        .into_iter()
        .map(|kind| {
            let path = format!("/run/d2b-diag-{}.json", kind.to_lowercase());
            (
                format!("{kind} rows"),
                format!(
                    "{} && jq -c '{CHAIN_PROJECTION}' {path} || true",
                    d2b(&format!("list {kind}"), &path)
                ),
            )
        })
        .collect::<Vec<_>>();
    dumps.push((
        "vms dir".to_owned(),
        "ls -la /run/d2b/vms/acceptance-guest/ 2>&1 || echo 'no vms dir'".to_owned(),
    ));
    dumps.push((
        "runtime tree".to_owned(),
        "ls -la /run/d2b/ /run/d2b/vms/ 2>&1 | head -n 40 || true".to_owned(),
    ));
    dumps.push((
        "worker argv".to_owned(),
        "ps -eo pid=,args= --no-headers 2>/dev/null | grep -F virtiofsd | head -n 10 \
         || echo 'no virtiofsd process'".to_owned(),
    ));
    dumps
}

/// The labelled pair as the diagnostics rows are passed.
fn rows(dumps: &[(String, String)]) -> Vec<DiagRow<'_>> {
    dumps
        .iter()
        .map(|(label, command)| (label.as_str(), command.as_str()))
        .collect()
}

/// The fixture's own `dump_rows`: announce the tag the failure is reported
/// under, and run every row dump under it.
fn dump_rows(control: &mut GuestControl, tag: &str, dumps: &[(String, String)]) {
    control.stage(tag);
    for (label, command) in dumps {
        control.diag(command, &format!("{tag}: {label}"));
    }
}

/// One sample of the teardown window: what each of the three lists held, and
/// whether the serving socket was still bound when they were read.
struct Sample {
    endpoint: u64,
    worker: u64,
    binding: u64,
    socket: bool,
}

impl Sample {
    /// Whether the chain is fully torn down in this sample.
    fn is_torn_down(&self) -> bool {
        self.binding == 0 && self.endpoint == 0 && self.worker == 0
    }

    /// The sample as the fixture's own `assert` messages rendered it: a
    /// Python dict, its keys in the order the jq program built them.
    fn as_python_dict(&self) -> String {
        format!(
            "{{'endpoint': {}, 'worker': {}, 'binding': {}, 'socket': {}}}",
            self.endpoint,
            self.worker,
            self.binding,
            if self.socket { "True" } else { "False" },
        )
    }

    /// The sample as the fixture's own `json.dumps` rendered it for the
    /// failure that reports a teardown which did not converge.
    fn as_json(&self) -> String {
        format!(
            "{{\"endpoint\": {}, \"worker\": {}, \"binding\": {}, \"socket\": {}}}",
            self.endpoint,
            self.worker,
            self.binding,
            if self.socket { "true" } else { "false" },
        )
    }
}

/// Read one sample out of what the sample command printed, the way the
/// fixture's own `json.loads` read it.
fn read_sample(output: &str) -> LegacyResult<Sample> {
    let value: Value = serde_json::from_str(output.trim()).map_err(|error| {
        LegacyError::Assertion(format!(
            "the teardown sample did not read as JSON: {error}: {}",
            output.trim()
        ))
    })?;
    let count = |name: &str| -> LegacyResult<u64> {
        value.get(name).and_then(Value::as_u64).ok_or_else(|| {
            LegacyError::Assertion(format!(
                "the teardown sample carried no {name} count: {value}"
            ))
        })
    };
    let socket = value
        .get("socket")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            LegacyError::Assertion(format!("the teardown sample carried no socket flag: {value}"))
        })?;
    Ok(Sample {
        endpoint: count("endpoint")?,
        worker: count("worker")?,
        binding: count("binding")?,
        socket,
    })
}

/// The wait between two samples of the teardown window, the fixture's own
/// `time.sleep(0.2)`: it ran on the driver, which is where a check's
/// assertions run, so it is a sleep on this thread rather than a command in
/// the guest.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn sleep_between_samples() {
    thread::sleep(SAMPLE_INTERVAL);
}
