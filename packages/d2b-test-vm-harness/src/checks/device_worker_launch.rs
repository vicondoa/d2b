//! The Device-worker launch path check (U17 slice 3), ported from its
//! fixture.
//!
//! It boots the daemon host the reusable node declares and proves, on a live
//! host, the path a Device's declared worker rows take. One Device declares
//! the Provider's swtpm rows and two declare the GPU rows; the Zone bundle
//! must carry exactly one declared row per Device worker template, owned by
//! its Device and bound to the Provider's digest-pinned executable with
//! launch arguments admitted, and the manager must ingest those rows so the
//! Process controller is the component that launches them.
//!
//! The TPM half runs end to end: `Process/swtpm-tpm0` reaches `Ready` with
//! the composed swtpm argv, the per-VM server socket and the
//! controller-created state dir exist, and the one-shot
//! `EphemeralProcess/swtpm-flush-tpm0` publishes its outcome as the row's
//! status projection. Deleting the Device then retires both rows and the
//! worker, children first.
//!
//! The GPU half is asserted the way the fixture asserted it, because the VM
//! has no GPU: both declared rows resolve and their launch is attempted, and
//! every GPU row must end on the named refusal
//! (`process-start-budget-exhausted` at `reconcile/launch`) - never `Ready`
//! and never a bare launch.
//!
//! The assertions below are the fixture's, in the fixture's order, with the
//! fixture's own command text and bounds. What the fixture expressed as
//! `machine.*` calls is [`GuestControl`]'s own operations, what it expressed
//! as the prelude's `stage`/`diag_unit`/`diag_wait` calls are the same
//! primitives, and what it built in Python - the `d2b`/`list_json`/
//! `flush_get` command builders and the row dumps its diagnostics print - are
//! private ports in this module, so a failure reported here reads the way the
//! fixture's failure read. The guest it boots is declared in
//! `nix/test-support/host-integration-node.nix`, and `start_all()` is not
//! restated here: it is the lane's own boot of the guest the check runs
//! against.

use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use crate::legacy::{DiagRow, GuestControl, LegacyError, LegacyResult};

/// The zone the check drives, the fixture's own.
const ZONE: &str = "work";

/// The Guest that owns the Devices under test, the fixture's own.
const GUEST: &str = "acceptance-guest";

/// The declared flush row, read by exact ref rather than by a zone-wide
/// `list EphemeralProcess`.
const FLUSH_ROW: &str = "swtpm-flush-tpm0";

/// The row uid the fixture requires: a real v4 uuid.
const UUID_V4: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";

/// The bound the daemon's activation gets, the fixture's own.
const DAEMON_UP: Duration = Duration::from_secs(180);

/// The bound the broker socket's wait gets, the fixture's own.
const BROKER_SOCKET: Duration = Duration::from_secs(30);

/// The bound the public socket's file wait gets, the fixture's own.
const PUBLIC_SOCKET: Duration = Duration::from_secs(30);

/// The bound the rows-ingested wait gets, the fixture's own.
const ROWS_INGESTED: Duration = Duration::from_secs(180);

/// The bound the declared TPM row's readiness wait gets, the fixture's own.
const TPM_READY: Duration = Duration::from_secs(180);

/// The bound the live swtpm process's wait gets, the fixture's own.
const TPM_WORKER: Duration = Duration::from_secs(60);

/// The bound the swtpm sockets' wait gets, the fixture's own.
const TPM_SOCKETS: Duration = Duration::from_secs(60);

/// The bound the flush row's outcome wait gets, the fixture's own.
const FLUSH_OUTCOME: Duration = Duration::from_secs(180);

/// The bound the teardown wait gets, the fixture's own.
const TEARDOWN: Duration = Duration::from_secs(180);

/// The window the launch-outcome evidence loop polls for, the fixture's own.
const OUTCOME_WINDOW: Duration = Duration::from_secs(150);

/// The window the GPU refusal loop polls for, the fixture's own.
const GPU_WINDOW: Duration = Duration::from_secs(180);

/// How long each of the two polling windows waits between attempts, the
/// fixture's own `_time.sleep(0.5)`.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The broker service the fixture starts by hand, after the broker socket.
const START_BROKER: &str = "systemctl start d2b-broker.service";

/// The Zone bundle every compile-time assertion in the first stage reads.
const BUNDLE: &str = "cat /etc/d2b/zones/work/resource-bundle.json";

/// The row fields the fixture's dumps and projections read, its own
/// `ROW_FIELDS`.
const ROW_FIELDS: &str = concat!(
    "{type: .type, name: .metadata.name, owner: .metadata.ownerRef, ",
    "uid: .metadata.uid, gen: .metadata.generation, ",
    "obs: .status.observedGeneration, phase: .status.phase, ",
    "template: .spec.template, resource: .status.resource}",
);

/// The dump the ingested Process rows are read out of.
const INGEST_PROCESS: &str = "/run/d2b-u17-ingest-process.json";

/// The dump the ingested flush row is read out of.
const INGEST_FLUSH: &str = "/run/d2b-u17-ingest-flush.json";

/// The dump the outcome window polls.
const OUTCOME_PROCESS: &str = "/run/d2b-u17-outcome-process.json";

/// The dump the outcome window's flush read polls.
const OUTCOME_FLUSH: &str = "/run/d2b-u17-outcome-flush.json";

/// The dump the GPU window polls.
const GPU_PROCESS: &str = "/run/d2b-u17-gpu-process.json";

/// The dump the TPM row's readiness wait reads.
const TPM_PROCESS: &str = "/run/d2b-u17-tpm-process.json";

/// The dump the flush row's outcome wait reads.
const FLUSH_PROCESS: &str = "/run/d2b-u17-flush-process.json";

/// The dump the teardown wait reads.
const TEARDOWN_PROCESS: &str = "/run/d2b-u17-teardown-process.json";

/// The dump the retired flush row is read out of.
const TEARDOWN_FLUSH: &str = "/run/d2b-u17-teardown-flush.json";

/// The dump the Device's revision is read out of.
const DEVICE_PRE_DELETE: &str = "/run/d2b-u17-device-pre-delete.json";

/// The dump the Device delete writes.
const DEVICE_DELETE: &str = "/run/d2b-u17-device-delete.json";

/// The resource types the first-stage diagnostics dump a row set for, the
/// fixture's own list.
const ROW_DUMP_KINDS: [&str; 5] = ["Device", "Process", "Endpoint", "Volume", "Provider"];

/// The `rows-ingested` wait's jq program, the fixture's own composition: three
/// declared Process rows, each with its Device owner, its declared template
/// and a real v4 uid, plus the flush row read by exact ref with the same
/// shape.
fn rows_ingested_jq() -> String {
    format!(
        concat!(
            "([.resources[] | select(.type == \"Process\" and ",
            "(.metadata.name | test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")))] ",
            "| length) == 3 and ",
            "([.resources[] | select(.type == \"Process\" and ",
            "(.metadata.name | test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) ",
            "and (.metadata.uid | test(\"{uuid}\")))] | length) == 3 and ",
            "([.resources[] | select(.type == \"Process\" and ",
            "({triples}))] | length) == 3 and ",
            "([$flush[0] | select(.type == \"EphemeralProcess\" and ",
            ".metadata.name == \"swtpm-flush-tpm0\" and ",
            ".metadata.ownerRef == \"Device/tpm0\" and ",
            ".spec.template == \"swtpm-init-flush\" and ",
            "(.metadata.uid | test(\"{uuid}\")))] | length) == 1",
        ),
        uuid = UUID_V4,
        triples = declared_process_triples(),
    )
}

/// The `Process/swtpm-tpm0` row must be `Ready` with its generation settled,
/// owned by its Device, on its declared template.
const TPM_ROW_READY: &str = concat!(
    "jq -e '([.resources[] | select(.type == \"Process\" and ",
    ".metadata.name == \"swtpm-tpm0\") | select(",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation and ",
    ".metadata.ownerRef == \"Device/tpm0\" and ",
    ".spec.template == \"swtpm-socket\")] | length) == 1' ",
    "/run/d2b-u17-tpm-process.json",
);

/// The swtpm worker really runs, under the Process controller.
const SWTPM_ALIVE: &str =
    "test \"$(ps -eo args= | awk '/[s]wtpm socket/ {c++} END {print c+0}')\" -ge 1";

/// The live swtpm workers' argv, one line each, the fixture's own read.
const SWTPM_ARGV: &str = concat!(
    "for pid in $(pgrep -f '[s]wtpm socket'); do tr '\\0' ' ' < /proc/$pid/cmdline; ",
    "echo; done",
);

/// The `--pid file=` value of the live swtpm worker, reduced to its
/// directory: the controller-created Volume root.
const SWTPM_STATE_DIR: &str = concat!(
    "for pid in $(pgrep -f '[s]wtpm socket'); do ",
    "tr '\\0' '\\n' < /proc/$pid/cmdline | sed -n '/--pid/{n;p}'; done ",
    "| head -n1 | sed 's|^file=||' | xargs -r dirname",
);

/// The one-shot flush publishes its outcome as the row's status projection,
/// which is what the TPM port's flush gate reads.
const FLUSH_OUTCOME_READY: &str = concat!(
    "jq -e '([select(.type == \"EphemeralProcess\" and ",
    ".metadata.name == \"swtpm-flush-tpm0\" and ",
    ".metadata.ownerRef == \"Device/tpm0\" and ",
    ".spec.template == \"swtpm-init-flush\" and ",
    ".status.resource.ephemeral.state == \"succeeded\" and ",
    ".status.resource.ephemeral.code == \"process-exited\")] | length) == 1' ",
    "/run/d2b-u17-flush-process.json",
);

/// The outcome window's row-status probe: the three declared Process rows
/// before the fixture's `---` marker, and the flush row after it.
const OUTCOME_PROBE: &str = concat!(
    "jq -c '[.resources[] | select(.metadata.name | ",
    "test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) | ",
    "{name: .metadata.name, phase: .status.phase, ",
    "resource: .status.resource}]' /run/d2b-u17-outcome-process.json",
    " && echo '---' && jq -c '[select(.metadata.name == ",
    "\"swtpm-flush-tpm0\") | {name: .metadata.name, ",
    "phase: .status.phase, resource: .status.resource}]' ",
    "/run/d2b-u17-outcome-flush.json",
);

/// The three declared Process rows as the outcome print reads them.
const OUTCOME_ROWS: &str = concat!(
    "jq -c '[.resources[] | select(.metadata.name | ",
    "test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) | ",
    "{name: .metadata.name, owner: .metadata.ownerRef, ",
    "phase: .status.phase, resource: .status.resource}]' ",
    "/run/d2b-u17-outcome-process.json",
);

/// The flush row as the outcome print reads it.
const OUTCOME_FLUSH_ROW: &str = concat!(
    "jq -c '[select(.metadata.name == \"swtpm-flush-tpm0\") | ",
    "{phase: .status.phase, resource: .status.resource}]' ",
    "/run/d2b-u17-outcome-flush.json",
);

/// The launch evidence the fixture prints from the daemon journal.
const LAUNCH_REFUSAL_LINES: &str = concat!(
    "journalctl -u d2bd.service --no-pager -o cat -b -n 4000 ",
    "| grep -E 'device-worker|process-resolution-refused|",
    "provider-ticket|swtpm|w1-gpu' | tail -n 40 || true",
);

/// The flush outcome projection the fixture prints.
const FLUSH_PROJECTION_ROWS: &str = "jq -c '[.status.resource]' /run/d2b-u17-flush-process.json";

/// Both GPU rows' phase and resource, grouped by Device.
const GPU_ROWS: &str = concat!(
    "jq -c '{gpu0: [.resources[] | select(.metadata.name == \"gpu-gpu0\") ",
    "| {phase: .status.phase, resource: .status.resource}], ",
    "gpu1: [.resources[] | select(.metadata.name == \"gpu-gpu1\") ",
    "| {phase: .status.phase, resource: .status.resource}]}' ",
    "/run/d2b-u17-gpu-process.json",
);

/// The GPU evidence the fixture prints from the daemon journal.
const GPU_REFUSAL_LINES: &str = concat!(
    "journalctl -u d2bd.service --no-pager -o cat -b -n 4000 ",
    "| grep -E 'device-worker|process-resolution-refused|gpu-runner-shape|",
    "render|w1-gpu|video' | tail -n 40 || true",
);

/// The Device's revision, which the delete carries.
const DEVICE_REVISION: &str = concat!(
    "jq -er '.resources[] | select(.type == \"Device\" and ",
    ".metadata.name == \"tpm0\") | .metadata.revision' ",
    "/run/d2b-u17-device-pre-delete.json",
);

/// Deleting the Device retires its declared rows through the Process
/// controller: the declared `Process/swtpm-tpm0` row is gone.
const TPM_ROWS_RETIRED: &str = concat!(
    "jq -e 'all(.resources[]; ",
    "(.type == \"Process\" and .metadata.name == \"swtpm-tpm0\") | not)' ",
    "/run/d2b-u17-teardown-process.json",
);

/// Whether a swtpm worker is still alive, without refusing when it is not.
const PGREP_SWTPM: &str = "pgrep -f '[s]wtpm socket' >/dev/null";

/// One declared Device worker row, the fixture's own `DECLARED_ROWS` entry:
/// its name, its resource type, its declared template, and the Device that
/// owns it.
struct DeclaredRow {
    name: &'static str,
    kind: &'static str,
    template: &'static str,
    owner: &'static str,
}

/// The declared rows, in the fixture's own order.
const DECLARED_ROWS: [DeclaredRow; 4] = [
    DeclaredRow {
        name: "swtpm-tpm0",
        kind: "Process",
        template: "swtpm-socket",
        owner: "Device/tpm0",
    },
    DeclaredRow {
        name: "swtpm-flush-tpm0",
        kind: "EphemeralProcess",
        template: "swtpm-init-flush",
        owner: "Device/tpm0",
    },
    DeclaredRow {
        name: "gpu-gpu0",
        kind: "Process",
        template: "gpu-worker",
        owner: "Device/gpu0",
    },
    DeclaredRow {
        name: "gpu-gpu1",
        kind: "Process",
        template: "gpu-worker",
        owner: "Device/gpu1",
    },
];

/// The template binding's owner is the Device *Provider* that signs the
/// template (the declared row's own owner is the Device), the fixture's own
/// `BINDING_OWNER`.
const BINDING_OWNERS: [(&str, &str); 3] = [
    ("Device/tpm0", "Provider/device-tpm"),
    ("Device/gpu0", "Provider/device-gpu"),
    ("Device/gpu1", "Provider/device-gpu"),
];

/// The worker executable each declared template names, the fixture's own
/// expected-binary table.
const EXPECTED_BINARIES: [(&str, &str); 5] = [
    ("swtpm-socket", "swtpm"),
    ("swtpm-init-flush", "swtpm-ioctl"),
    ("gpu-worker", "crosvm"),
    ("gpu-render-node", "crosvm"),
    ("video-worker", "crosvm"),
];

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    let dumps = row_dumps();
    let rows = as_rows(&dumps);

    control.stage("daemon-up");
    control.diag_unit("daemon-up", "d2bd.service", DAEMON_UP)?;
    control.wait_for_unit("d2b-broker.socket", None, BROKER_SOCKET)?;
    control.wait_for_file("/run/d2b/public.sock", PUBLIC_SOCKET)?;
    control.succeed(&[START_BROKER], None)?;

    // 1. Slice 1's compile-time half, on the live host: the Zone bundle
    //    carries exactly one declared row per Device worker template, owned by
    //    its Device, each bound to the Provider's digest-pinned executable
    //    with launch arguments admitted. No hardware and no launch involved.
    control.stage("bundle-projection");
    let bundle = parse_json(
        &control.succeed(&[BUNDLE], None)?,
        "the Zone resource bundle",
    )?;
    let declared = declared_rows(&bundle);
    for declared_row in &DECLARED_ROWS {
        let (name, kind, template, owner) = (
            declared_row.name,
            declared_row.kind,
            declared_row.template,
            declared_row.owner,
        );
        let Some(row) = declared.get(&(kind, name)) else {
            return Err(LegacyError::Assertion(format!(
                "declared row {kind}/{name} missing from the bundle"
            )));
        };
        let declared_owner = row
            .get("metadata")
            .and_then(|metadata| metadata.get("ownerRef"));
        if declared_owner.and_then(Value::as_str) != Some(owner) {
            return Err(LegacyError::Assertion(format!(
                "{kind}/{name}: declared owner {} != {}",
                python_repr(declared_owner),
                python_repr_str(owner),
            )));
        }
        let declared_template = row.get("spec").and_then(|spec| spec.get("template"));
        if declared_template.and_then(Value::as_str) != Some(template) {
            return Err(LegacyError::Assertion(format!(
                "{kind}/{name}: declared template {} != {}",
                python_repr(declared_template),
                python_repr_str(template),
            )));
        }
        if declares_argv(row) {
            return Err(LegacyError::Assertion(format!(
                "{kind}/{name}: the declared row must stay argv-free"
            )));
        }
    }
    let bindings = template_bindings(&bundle);
    let compiled = process_templates(&bundle)
        .iter()
        .map(|binding| {
            json!({
                "processRef": binding.get("processRef").cloned().unwrap_or(Value::Null),
                "ownerRef": binding.get("ownerRef").cloned().unwrap_or(Value::Null),
                "template": binding.get("template").cloned().unwrap_or(Value::Null),
                "binaryRef": binding.get("binaryRef").cloned().unwrap_or(Value::Null),
                "launchArgs": binding.get("launchArgs").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .collect::<Vec<_>>();
    control.announce(&format!(
        "[d2b] compiled processTemplates: {}",
        python_dumps(&Value::Array(compiled)),
    ));
    for declared_row in &DECLARED_ROWS {
        let (name, kind, template, owner) = (
            declared_row.name,
            declared_row.kind,
            declared_row.template,
            declared_row.owner,
        );
        let reference = format!("{kind}/{name}");
        let Some(binding) = bindings.get(reference.as_str()) else {
            return Err(LegacyError::Assertion(format!(
                "no Device worker template binding for {reference}"
            )));
        };
        let binding_template = binding.get("template");
        if binding_template.and_then(Value::as_str) != Some(template) {
            return Err(LegacyError::Assertion(format!(
                "{reference}: binding template {} != {}",
                python_repr(binding_template),
                python_repr_str(template),
            )));
        }
        let launch_args = binding.get("launchArgs");
        if launch_args != Some(&Value::Bool(true)) {
            return Err(LegacyError::Assertion(format!(
                "{reference}: binding must admit launch arguments (saw {})",
                python_repr(launch_args),
            )));
        }
        let expected_owner = binding_owner(owner);
        let binding_owner_ref = binding.get("ownerRef");
        if binding_owner_ref.and_then(Value::as_str) != Some(expected_owner) {
            return Err(LegacyError::Assertion(format!(
                "{reference}: binding owner {} != {}",
                python_repr(binding_owner_ref),
                python_repr_str(expected_owner),
            )));
        }
        let expected = expected_binary(template);
        let binding_binary = binding.get("binaryRef");
        if binding_binary.and_then(Value::as_str) != Some(expected) {
            return Err(LegacyError::Assertion(format!(
                "{reference}: binding binary {} != {}",
                python_repr(binding_binary),
                python_repr_str(expected),
            )));
        }
    }
    let declared_bindings = bindings
        .iter()
        .map(|(reference, binding)| {
            json!({
                "processRef": reference,
                "template": binding.get("template").cloned().unwrap_or(Value::Null),
                "binaryRef": binding.get("binaryRef").cloned().unwrap_or(Value::Null),
                "launchArgs": binding.get("launchArgs").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .collect::<Vec<_>>();
    control.announce(&format!(
        "[d2b] declared device worker bindings: {}",
        python_dumps(&Value::Array(declared_bindings)),
    ));

    // 2. The rows are ingested into the manager with their Device owner, so the
    //    Process controller is the component that launches them (KTD13). The
    //    three declared Process rows are the whole Process set this stage
    //    asserts (the fourth declared row is the flush EphemeralProcess below),
    //    each present with its Device owner, declared template, and a real v4
    //    uid.
    control.stage("rows-ingested");
    control.diag_wait(
        "rows-ingested",
        &rows_ingested_command(),
        ROWS_INGESTED,
        &rows,
        &[("d2bd.service", "device-worker")],
    )?;
    for declared_row in &DECLARED_ROWS {
        let (name, kind, template, owner) = (
            declared_row.name,
            declared_row.kind,
            declared_row.template,
            declared_row.owner,
        );
        let (source, row_source) = if kind == "Process" {
            (
                INGEST_PROCESS,
                format!(".resources[] | select(.type == \"{kind}\" and .metadata.name == \"{name}\")"),
            )
        } else {
            (INGEST_FLUSH, ".".to_owned())
        };
        let output = control.succeed(
            &[&format!(
                "jq -c '[{row_source} | {{owner: .metadata.ownerRef, \
                 template: .spec.template, phase: .status.phase}}]' {source}"
            )],
            None,
        )?;
        let ingested = parse_json(&output, "the ingested rows dump")?;
        let ingested: &[Value] = match ingested.as_array() {
            Some(ingested) => ingested.as_slice(),
            None => &[],
        };
        if ingested.len() != 1 {
            return Err(LegacyError::Assertion(format!(
                "{kind}/{name}: expected one ingested row, got {output}"
            )));
        }
        let ingested_owner = ingested[0].get("owner");
        let ingested_template = ingested[0].get("template");
        if ingested_owner.and_then(Value::as_str) != Some(owner)
            || ingested_template.and_then(Value::as_str) != Some(template)
        {
            return Err(LegacyError::Assertion(format!(
                "{kind}/{name}: ingested shape {output}"
            )));
        }
    }

    // Evidence for the read path above (not an assertion): a zone-wide
    // `list EphemeralProcess` is exec-routed and never reaches the manager, so
    // record its exit status and stderr once per run. The declared row itself
    // is read by exact ref through `flush_get` instead.
    let list_probe = control.execute(
        &d2b(
            "list EphemeralProcess 2>&1",
            "/run/d2b-u17-ephemeralprocess-probe.json",
        ),
        None,
    )?;
    control.announce(&format!(
        "[d2b] zone-wide list EphemeralProcess probe: exit {}; {}",
        list_probe.status,
        list_probe.output.trim().replace('\n', " | "),
    ));

    // 3. Launch-outcome evidence (diagnostic, not an assertion): every
    //    declared row reaches either Ready or a terminal classification within
    //    the bounded window, and the log carries the classification the row
    //    and the daemon publish. A row still Pending here is a launch that
    //    never resolved; the stages below assert the target behavior.
    control.stage("worker-launch-outcome");
    let outcome_deadline = Instant::now() + OUTCOME_WINDOW;
    while Instant::now() < outcome_deadline {
        control.succeed(&[&format!("{}; echo OK", list_json("Process", OUTCOME_PROCESS))], None)?;
        control.succeed(&[&format!("{}; echo OK", flush_get(OUTCOME_FLUSH))], None)?;
        let probed = control.succeed(&[OUTCOME_PROBE], None)?;
        // The fixture read the half before its `---` marker, which is the
        // three declared Process rows.
        let rows_now = parse_json(
            probed.split("---").next().unwrap_or_default(),
            "the declared row outcomes",
        )?;
        let flat_now: &[Value] = match rows_now.as_array() {
            Some(rows_now) => rows_now.as_slice(),
            None => &[],
        };
        let settled = flat_now.iter().filter(|row| is_settled(row)).count();
        if !flat_now.is_empty() && settled == flat_now.len() {
            break;
        }
        pause();
    }
    let declared_outcomes = parse_json(
        &control.succeed(&[OUTCOME_ROWS], None)?,
        "the declared row outcomes",
    )?;
    if let Some(declared_outcomes) = declared_outcomes.as_array() {
        for row in declared_outcomes {
            control.announce(&format!("[d2b] declared row outcome: {}", python_dumps(row)));
        }
    }
    let flush_outcome = control.succeed(&[OUTCOME_FLUSH_ROW], None)?;
    control.announce(&format!("[d2b] flush row outcome: {flush_outcome}"));
    let launch_lines = control.succeed(&[LAUNCH_REFUSAL_LINES], None)?;
    control.announce(&format!("[d2b] launch refusal lines:\n{launch_lines}"));

    // 4. TPM end to end. The declared `Process/swtpm-tpm0` row is launched by
    //    the Process controller with the parameters the Device row, the
    //    declared template, and the daemon runtime paths supply: it reaches
    //    Ready, the real swtpm process is alive with that argv, and its
    //    sockets exist.
    control.stage("tpm-worker");
    control.diag_wait(
        "tpm-worker-ready",
        &format!("{} && {TPM_ROW_READY}", list_json("Process", TPM_PROCESS)),
        TPM_READY,
        &rows,
        &[("d2bd.service", "swtpm"), ("d2b-broker.service", "w1-swtpm")],
    )?;
    control.diag_wait(
        "tpm-worker-process",
        SWTPM_ALIVE,
        TPM_WORKER,
        &rows,
        &[("d2bd.service", "swtpm")],
    )?;
    let argv = control.succeed(&[SWTPM_ARGV], None)?.trim().to_owned();
    control.announce(&format!("[d2b] live swtpm argv: {argv}"));
    if !(argv.contains("--tpm2")
        && argv.contains("--ctrl")
        && argv.contains("--server")
        && argv.contains("--tpmstate"))
    {
        return Err(LegacyError::Assertion(format!(
            "the live swtpm argv must be the composed swtpm shape: {}",
            python_repr_str(&argv),
        )));
    }
    if !argv.contains(&format!("path=/run/d2b/vms/{GUEST}/tpm.sock")) {
        return Err(LegacyError::Assertion(format!(
            "the live swtpm argv must carry the per-VM server socket: {}",
            python_repr_str(&argv),
        )));
    }
    if !(argv.contains("device-") && argv.contains("tpm-state/ctrl.sock")) {
        return Err(LegacyError::Assertion(format!(
            "the live swtpm argv must carry the controller-created state dir: {}",
            python_repr_str(&argv),
        )));
    }
    let state_dir = control.succeed(&[SWTPM_STATE_DIR], None)?.trim().to_owned();
    if !(state_dir.starts_with("/var/lib/d2b/") && state_dir.ends_with("tpm-state")) {
        return Err(LegacyError::Assertion(format!(
            "the swtpm state dir must be the controller-created Volume root: {}",
            python_repr_str(&state_dir),
        )));
    }
    control.diag_wait(
        "tpm-sockets",
        &format!("test -S /run/d2b/vms/{GUEST}/tpm.sock && test -S {state_dir}/ctrl.sock"),
        TPM_SOCKETS,
        &rows,
        &[("d2bd.service", "swtpm")],
    )?;
    let sockets = control.succeed(
        &[&format!(
            "stat -c '%F %a %U:%G %n' /run/d2b/vms/{GUEST}/tpm.sock {state_dir}/ctrl.sock"
        )],
        None,
    )?;
    control.announce(&format!(
        "[d2b] swtpm state dir: {state_dir}; sockets: {}",
        sockets.replace('\n', " | "),
    ));

    // 4. The one-shot flush publishes its outcome as the row's status
    //    projection, which is what the TPM port's flush gate reads.
    control.stage("tpm-flush");
    control.diag_wait(
        "tpm-flush-outcome",
        &format!("{} && {FLUSH_OUTCOME_READY}", flush_get(FLUSH_PROCESS)),
        FLUSH_OUTCOME,
        &rows,
        &[
            ("d2bd.service", "swtpm-flush"),
            ("d2bd.service", "ephemeral"),
        ],
    )?;
    let flush_projection = control.succeed(&[FLUSH_PROJECTION_ROWS], None)?;
    control.announce(&format!("[d2b] flush outcome projection: {flush_projection}"));

    // 5. GPU: the launch path, honestly. The VM has no GPU, so the fixture
    //    states what must happen instead of faking a device: the declared rows
    //    resolve, their launch is attempted through the Process controller and
    //    the broker, and every GPU row ends on a named refusal - never Ready,
    //    never a bare launch.
    control.stage("gpu-launch");
    // The GPU rows must end on a named refusal, never Ready and never a fake
    // device. The row status (and the daemon journal) name the stage.
    let mut observed: Option<Value> = None;
    let deadline = Instant::now() + GPU_WINDOW;
    while Instant::now() < deadline {
        control.succeed(&[&format!("{}; echo OK", list_json("Process", GPU_PROCESS))], None)?;
        let statuses = parse_json(&control.succeed(&[GPU_ROWS], None)?, "the GPU row statuses")?;
        let flat = gpu_rows(&statuses);
        let terminal = flat
            .iter()
            .filter(|row| matches!(row_phase(row), Some("Failed" | "Quarantined")))
            .count();
        if terminal == 2 || flat.iter().any(|row| row_phase(row) == Some("Ready")) {
            observed = Some(statuses);
            break;
        }
        pause();
    }
    let Some(observed) = observed else {
        return Err(LegacyError::Assertion(
            "every GPU/video row must reach a terminal refusal without a GPU".to_owned(),
        ));
    };
    let flat = gpu_rows(&observed);
    if flat.len() != 2 || !flat.iter().all(|row| row_phase(row) == Some("Failed")) {
        return Err(LegacyError::Assertion(format!(
            "every GPU/video row must end Failed without a GPU: {}",
            python_dumps(&observed),
        )));
    }
    for row in &flat {
        // The refusal is the closed classification the plan documents: the
        // restart ceiling makes a persistently refused launch terminal as
        // `process-start-budget-exhausted` at the launch stage
        // (`reconcile/launch`), never Ready and never a bare failure.
        let failure = row
            .get("resource")
            .and_then(|resource| resource.get("driverFailure"));
        let code = failure
            .and_then(|failure| failure.get("code"))
            .and_then(Value::as_str);
        let stage = failure
            .and_then(|failure| failure.get("stage"))
            .and_then(Value::as_str);
        if code != Some("process-start-budget-exhausted") || stage != Some("reconcile/launch") {
            return Err(LegacyError::Assertion(format!(
                "the GPU/video refusal must be the budget-exhausted launch refusal \
                 (process-start-budget-exhausted at reconcile/launch): {}",
                python_dumps(row),
            )));
        }
    }
    control.announce(&format!(
        "[d2b] GPU/video terminal refusals: {}",
        python_dumps(&observed),
    ));
    let codes = control.succeed(&[GPU_REFUSAL_LINES], None)?;
    control.announce(&format!("[d2b] GPU/video refusal evidence:\n{codes}"));

    // 6. Teardown: deleting the Device retires its declared rows through the
    //    Process controller - the process stops and the rows go away, children
    //    first, with nothing of the Device left behind.
    control.stage("tpm-teardown");
    control.succeed(&[&list_json("Device", DEVICE_PRE_DELETE)], None)?;
    let revision = control.succeed(&[DEVICE_REVISION], None)?.trim().to_owned();
    control.succeed(
        &[&d2b(&format!("delete Device/tpm0 --revision {revision}"), DEVICE_DELETE)],
        None,
    )?;
    control.diag_wait(
        "tpm-teardown",
        &format!("{} && {TPM_ROWS_RETIRED}", list_json("Process", TEARDOWN_PROCESS)),
        TEARDOWN,
        &rows,
        &[("d2bd.service", "swtpm"), ("d2bd.service", "delete")],
    )?;
    let flush_gone = control.execute(&flush_get(TEARDOWN_FLUSH), None)?;
    if flush_gone.status == 0 {
        return Err(LegacyError::Assertion(format!(
            "the Device delete must retire EphemeralProcess/swtpm-flush-tpm0 \
             (Get must refuse, saw exit {})",
            flush_gone.status,
        )));
    }
    control.announce(&format!(
        "[d2b] flush row after teardown: {}",
        flush_gone.output.trim().replace('\n', " | "),
    ));
    if control.execute(PGREP_SWTPM, None)?.status == 0 {
        return Err(LegacyError::Assertion(
            "no swtpm worker may outlive the Device that declared it".to_owned(),
        ));
    }
    control.announce("[d2b] Device/tpm0 teardown retired its declared rows and the worker");

    control.stage("done");
    control.announce("[d2b] U17 device-worker launch path holds on the live host");
    Ok(())
}

/// The public CLI invocation every read in this check makes, the fixture's
/// own `d2b(command, out)`.
fn d2b(command: &str, out: &str) -> String {
    format!(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock \
         d2b --zone {ZONE} --json {command} >{out}"
    )
}

/// One zone-wide list of a resource type, the fixture's own `list_json`.
fn list_json(kind: &str, out: &str) -> String {
    d2b(&format!("list {kind}"), out)
}

/// The declared flush row read by exact ref, the fixture's own `flush_get`.
fn flush_get(out: &str) -> String {
    d2b(&format!("get EphemeralProcess/{FLUSH_ROW}"), out)
}

/// The bundle's declared rows by `(type, name)`, the fixture's own `declared`
/// map: the Process and EphemeralProcess rows the Zone bundle carries.
fn declared_rows(bundle: &Value) -> BTreeMap<(&str, &str), &Value> {
    let mut declared = BTreeMap::new();
    for row in bundle_resources(bundle) {
        let kind = row.get("type").and_then(Value::as_str);
        let name = row
            .get("metadata")
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str);
        if let (Some(kind), Some(name)) = (kind, name)
            && matches!(kind, "Process" | "EphemeralProcess")
        {
            declared.insert((kind, name), row);
        }
    }
    declared
}

/// The bundle's `resources`, the rows its declared map is built from.
fn bundle_resources(bundle: &Value) -> &[Value] {
    match bundle.get("resources").and_then(Value::as_array) {
        Some(resources) => resources.as_slice(),
        None => &[],
    }
}

/// The bundle's `processTemplates`, or none when the bundle carries none.
fn process_templates(bundle: &Value) -> &[Value] {
    match bundle.get("processTemplates").and_then(Value::as_array) {
        Some(bindings) => bindings.as_slice(),
        None => &[],
    }
}

/// The bundle's process template bindings by `processRef`, the fixture's own
/// `bindings` map.
fn template_bindings(bundle: &Value) -> BTreeMap<&str, &Value> {
    process_templates(bundle)
        .iter()
        .filter_map(|binding| {
            binding
                .get("processRef")
                .and_then(Value::as_str)
                .map(|process_ref| (process_ref, binding))
        })
        .collect()
}

/// Whether a declared row carries launch arguments of its own, which the
/// fixture refused: a declared row names a template, never an argv.
fn declares_argv(row: &Value) -> bool {
    match row.get("spec").and_then(Value::as_object) {
        Some(spec) => spec.contains_key("command") || spec.contains_key("argv"),
        None => false,
    }
}

/// The `or`-joined triples the rows-ingested wait requires, the fixture's own
/// composition over the declared Process rows.
fn declared_process_triples() -> String {
    DECLARED_ROWS
        .iter()
        .filter(|row| row.kind == "Process")
        .map(|row| {
            format!(
                "(.metadata.name == \"{}\" and .metadata.ownerRef == \"{}\" \
                 and .spec.template == \"{}\")",
                row.name, row.owner, row.template,
            )
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

/// The rows-ingested wait's command, the fixture's own: the Process list, the
/// flush row read by exact ref, and the jq program that asserts both.
fn rows_ingested_command() -> String {
    format!(
        "{} && {} && jq -e --slurpfile flush /run/d2b-u17-ingest-flush.json '{}' \
         /run/d2b-u17-ingest-process.json",
        list_json("Process", INGEST_PROCESS),
        flush_get(INGEST_FLUSH),
        rows_ingested_jq(),
    )
}

/// Both Devices' row arrays, concatenated the way the fixture's `flat` was.
fn gpu_rows(observed: &Value) -> Vec<&Value> {
    let mut rows = Vec::new();
    for key in ["gpu0", "gpu1"] {
        if let Some(array) = observed.get(key).and_then(Value::as_array) {
            rows.extend(array.iter());
        }
    }
    rows
}

/// A row's phase, or nothing when it carries none.
fn row_phase(row: &Value) -> Option<&str> {
    row.get("phase").and_then(Value::as_str)
}

/// Whether a row is out of the launch path: `Ready`, or terminally Failed or
/// Quarantined.
fn is_settled(row: &Value) -> bool {
    matches!(row_phase(row), Some("Ready" | "Failed" | "Quarantined"))
}

/// The executable a declared template names. Every template this fixture
/// declares on a Device is in the table; a template outside it is reported as
/// the empty executable, so the binding assertion below fails honestly.
fn expected_binary(template: &str) -> &str {
    EXPECTED_BINARIES
        .iter()
        .find(|(declared, _)| *declared == template)
        .map(|(_, binary)| *binary)
        .unwrap_or("")
}

/// The Device *Provider* that signs one Device's templates, the fixture's own
/// `BINDING_OWNER` lookup.
fn binding_owner(owner: &str) -> &str {
    BINDING_OWNERS
        .iter()
        .find(|(device, _)| *device == owner)
        .map(|(_, provider)| *provider)
        .unwrap_or("")
}

/// One command's JSON output, as the fixture's `_json.loads` read it.
fn parse_json(output: &str, what: &str) -> LegacyResult<Value> {
    serde_json::from_str(output)
        .map_err(|error| LegacyError::Assertion(format!("{what} is not readable JSON: {error}")))
}

/// The interval the fixture's two bounded polling windows slept between
/// attempts: `_time.sleep(0.5)`, from the host that drives the guest.
///
/// The lint's replacement vocabulary is for executor workers; a ported
/// check's assertions run on the lane's own synchronous check path, which is
/// the path every [`GuestControl`] operation runs on, so this takes the
/// per-site allow with the sanctioned reason rather than an async timer.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn pause() {
    thread::sleep(POLL_INTERVAL);
}

/// The rows the fixture's diagnostics dump on a timed-out wait, in its own
/// order: one per resource type, the exec-routed flush/list probe, the zone
/// bundle, the storage rows, the site artifact, and the worker sockets.
fn row_dumps() -> Vec<(String, String)> {
    let row_projection = format!("[.resources[] | {ROW_FIELDS}]");
    let flush_projection = format!("[. | {ROW_FIELDS}]");
    let mut dumps = Vec::new();
    for kind in ROW_DUMP_KINDS {
        let out = format!("/run/d2b-u17-{}.json", kind.to_lowercase());
        dumps.push((
            format!("{kind} rows"),
            format!(
                "{} && jq -c '{row_projection}' {out} || true",
                list_json(kind, &out),
            ),
        ));
    }
    dumps.push((
        "declared flush row (Get) and zone-wide list EphemeralProcess (exec-routed)".to_owned(),
        format!(
            "{} && jq -c '{flush_projection}' \
             /run/d2b-u17-ephemeralprocess-get.json; rc=0; {} \
             || rc=$?; echo list-exit=$rc; true",
            flush_get("/run/d2b-u17-ephemeralprocess-get.json"),
            d2b(
                "list EphemeralProcess 2>&1",
                "/run/d2b-u17-ephemeralprocess.json",
            ),
        ),
    ));
    dumps.push((
        "zone resource bundle".to_owned(),
        concat!(
            "jq -c '{resources: [.resources[] | select(.type == \"Process\" or ",
            ".type == \"EphemeralProcess\") | {type, name: .metadata.name, ",
            "owner: .metadata.ownerRef, template: .spec.template}], ",
            "bindings: [.processTemplates[] | {processRef, ownerRef, template, ",
            "launchArgs, binaryRef}]}' /etc/d2b/zones/work/resource-bundle.json ",
            "|| true",
        )
        .to_owned(),
    ));
    dumps.push((
        "swtpm storage rows".to_owned(),
        concat!(
            "jq -c '[.paths[] | select(.id | test(\"swtpm\")) | {id, scope, ",
            "pathTemplate}]' /etc/d2b/storage.json || true",
        )
        .to_owned(),
    ));
    // The two accounts the journal and the row dumps between them could not
    // give. The swtpm storage rows above filter on `swtpm`, which by
    // construction cannot show the per-guest runtime dir a device worker
    // binds its socket under - so a guest that has that row and a guest that
    // does not read identically here. And the broker's audit log records the
    // spawn decisions the journal only summarises, so when the row sits
    // Pending with `w1-swtpm` matching nothing in the journal, this is what
    // says whether the swtpm-dir fence and the spawn plan ran at all.
    dumps.push((
        "per-guest runtime dir storage row and the broker's spawn audit".to_owned(),
        concat!(
            "jq -c '[.paths[] | select(.id | test(\"vm-run\")) | {id, scope, ",
            "pathTemplate, mode, owner, group, creator}]' /etc/d2b/storage.json ",
            "|| echo 'no vm-run row in the storage contract'; echo '--- audit ---'; ",
            "grep -hoE '\"(event|op|kind)\":\"[^\"]*\"' /var/lib/d2b/audit/broker-*.jsonl ",
            "2>/dev/null | sort | uniq -c | sort -rn | head -25; echo '--- swtpm audit ---'; ",
            "grep -h 'swtpm\\|SpawnRunner\\|PrepareSwtpm' /var/lib/d2b/audit/broker-*.jsonl ",
            "2>/dev/null | tail -20 || echo 'no swtpm record in the broker audit log'; true",
        )
        .to_owned(),
    ));
    dumps.push((
        "site artifact".to_owned(),
        "cat /etc/d2b/site.json 2>/dev/null || echo 'no site.json'".to_owned(),
    ));
    dumps.push((
        "device worker sockets".to_owned(),
        concat!(
            "find /run/d2b/vms /run/d2b-video -maxdepth 3 ",
            "-printf '%M %u:%g %p\\n' 2>/dev/null | sort | head -n 40 || true",
        )
        .to_owned(),
    ));
    dumps
}

/// The row dumps as the diagnostics take them.
fn as_rows(dumps: &[(String, String)]) -> Vec<DiagRow<'_>> {
    dumps
        .iter()
        .map(|(label, command)| (label.as_str(), command.as_str()))
        .collect()
}

/// `json.dumps(value, sort_keys=True)`, the rendering the fixture's `print`
/// lines used: keys sorted, `, ` and `: ` separators, and non-ASCII escaped
/// the way `ensure_ascii` escapes it.
fn python_dumps(value: &Value) -> String {
    let mut out = String::new();
    dumps_into(value, &mut out);
    out
}

/// One value appended to a `json.dumps` rendering.
fn dumps_into(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => dumps_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dumps_into(item, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            let mut keys = fields.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dumps_string(key, out);
                out.push_str(": ");
                dumps_into(&fields[(*key).as_str()], out);
            }
            out.push('}');
        }
    }
}

/// One string in `json.dumps` form: double-quoted, with the escapes
/// `ensure_ascii` writes.
fn dumps_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            character if !(' '..='~').contains(&character) => {
                let mut buffer = [0_u16; 2];
                for unit in character.encode_utf16(&mut buffer) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

/// `repr(...)` of a value the fixture interpolated with `!r`; a field a row
/// does not carry is `None`.
fn python_repr(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_owned(),
        Some(Value::Bool(true)) => "True".to_owned(),
        Some(Value::Bool(false)) => "False".to_owned(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => python_repr_str(text),
        Some(Value::Array(items)) => {
            let items = items
                .iter()
                .map(|item| python_repr(Some(item)))
                .collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        }
        Some(Value::Object(fields)) => {
            let fields = fields
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", python_repr_str(key), python_repr(Some(value)))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

/// `repr(...)` of a string, the way the fixture's `!r` interpolations
/// rendered one.
fn python_repr_str(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for character in text.chars() {
        if character == quote {
            out.push('\\');
            out.push(character);
        } else {
            match character {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                character if character < ' ' => {
                    out.push_str(&format!("\\x{:02x}", character as u32));
                }
                character => out.push(character),
            }
        }
    }
    out.push(quote);
    out
}
