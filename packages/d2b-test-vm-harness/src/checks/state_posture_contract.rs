//! The declared host posture contract for state farms and shared
//! directories, ported from its fixture (issue #512).
//!
//! One file declares the posture
//! (`packages/d2b-broker/src/ops/state-posture-contract.json`); the broker
//! posture code embeds it, the Nix provisioning derives from it, and this
//! check asserts the live host against it: every declared level's
//! owner/group/mode/ACL, plus the allowed and denied operations for each
//! principal (root, d2bd, a d2b-group launcher, and nobody). The fixture
//! never restated a posture value - it read the declaration - so neither does
//! this module: the contract is parsed at run time and the same tree and
//! level walk is performed over it.
//!
//! What the fixture expressed as its own Python helpers comes with it, as
//! private functions of this module: `substitute` (the `<state-root>` /
//! `<zone>` / `<guest>` / `<vm>` tokens), `tree`, `level_path`, `stat_row`,
//! `acl_entries` (with the fixture's observation cache), `named_entry`,
//! `permission_bits`, `level_exists`, `spawn_preflight_entries` (the
//! structural carve-out for the broker's spawn-time traversal grants, which
//! cannot be declared because the runner uids are minted per guest at run
//! time), `run_as`, `probe` and `check_level`. Every message is the
//! fixture's own, because a drift report that reads differently from the
//! fixture's report is a differently reported drift.
//!
//! The guest boots the same Zone-native Cloud Hypervisor Guest recipe the
//! acceptance fixture uses - the writable-store shape plus the acceptance
//! artifacts, zones and guest system - so the state chain, the store-view
//! farm, and the spawn-time traversal ACLs are all materialized by the
//! product path. That guest, and the `let` bindings that build it, are
//! declared in `nix/test-support/host-integration-node.nix`.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::{collections::{BTreeMap, BTreeSet}, time::Duration};

use serde_json::Value;

use crate::legacy::{shlex_quote, DiagRow, GuestControl, LegacyError, LegacyResult};

/// The zone the check drives.
const ZONE: &str = "work";

/// The guest inside that zone whose state chain is checked.
const GUEST: &str = "acceptance-guest";

/// The state root every declared path hangs off.
const STATE_ROOT: &str = "/var/lib/d2b";

/// The bound the daemon's activation gets, the fixture's own.
const DAEMON_UP: Duration = Duration::from_secs(180);

/// The bound the broker socket gets, the fixture's own.
const BROKER_SOCKET: Duration = Duration::from_secs(30);

/// The bound the public socket's file wait gets, the fixture's own.
const PUBLIC_SOCKET: Duration = Duration::from_secs(30);

/// The bound the broker service's activation gets, the fixture's own.
const BROKER_SERVICE: Duration = Duration::from_secs(30);

/// The bound each of the two store-view waits gets, the fixture's own.
const STORE_WAIT: Duration = Duration::from_secs(300);

/// The principals whose declared rights are probed, and the linux user each
/// one is.
const PRINCIPAL_USER: [(&str, &str); 4] = [
    ("root", "root"),
    ("d2bd", "d2bd"),
    ("d2b", "alice"),
    ("nobody", "nobody"),
];

/// The contract trees the check compares the live host with.
const CHECKED_TREES: [&str; 5] = [
    "state-root",
    "guest-state-chain",
    "guest-state-dir",
    "guest-store-view",
    "shared-run-dir",
];

/// One `stat -c` row of a declared level.
struct Stat {
    owner: String,
    group: String,
    mode: String,
}

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("daemon-up");

    // The declared contract, from the same file the broker posture code
    // embeds and `nixos-modules/host-daemon.nix` derives its provisioning
    // from.
    let contract_text = control.succeed(&["cat /etc/d2b/state-posture-contract.json"], None)?;
    let contract: Value = serde_json::from_str(&contract_text).map_err(|error| {
        LegacyError::Assertion(format!(
            "the state posture contract is not readable JSON: {error}"
        ))
    })?;

    control.diag_unit("daemon-up", "d2bd.service", DAEMON_UP)?;
    control.wait_for_unit("d2b-broker.socket", None, BROKER_SOCKET)?;
    control.wait_for_file("/run/d2b/public.sock", PUBLIC_SOCKET)?;
    control.succeed(&["systemctl start d2b-broker.service"], None)?;
    control.diag_unit("broker-service", "d2b-broker.service", BROKER_SERVICE)?;

    // The ids each principal's probes run as.
    let mut principal_ids = BTreeMap::new();
    for (principal, user) in PRINCIPAL_USER {
        let uid = control
            .succeed(&[&format!("id -u {user}")], None)?
            .trim()
            .to_owned();
        let gid = control
            .succeed(&[&format!("id -g {user}")], None)?
            .trim()
            .to_owned();
        principal_ids.insert(principal.to_owned(), (uid, gid));
    }

    let guest_state = format!("{STATE_ROOT}/zones/{ZONE}/guests/{GUEST}");
    let store_view = format!("{guest_state}/store-view");

    // The store-view farm syncs behind the guest start, and the vmm binds the
    // guest's own socket.
    control.stage("store-view-sync");
    let guest_state_row = (
        "guest state tree".to_owned(),
        format!(
            "find {guest_state} -maxdepth 1 -exec stat -c '%A %U %G %n' {{}} + 2>/dev/null | sort || true"
        ),
    );
    let store_view_row = (
        "store-view tree".to_owned(),
        format!(
            "find {store_view} -maxdepth 2 -exec stat -c '%A %U %G %n' {{}} + 2>/dev/null | sort || true"
        ),
    );
    let store_view_command = format!(
        "test -L {store_view}/state/current && test -L {store_view}/meta/current"
    );
    control.diag_wait(
        "store-view-sync",
        &store_view_command,
        STORE_WAIT,
        &[as_row(&guest_state_row), as_row(&store_view_row)],
        &[
            ("d2bd.service", "store"),
            ("d2b-broker.service", "StoreSync"),
        ],
    )?;

    control.stage("vmm-spawn");
    let vmm_socket_command = format!("test -S {guest_state}/{GUEST}.sock");
    control.diag_wait(
        "vmm-spawn",
        &vmm_socket_command,
        STORE_WAIT,
        &[as_row(&guest_state_row)],
        &[
            ("d2bd.service", "cloud-hypervisor"),
            ("d2bd.service", "component-session"),
        ],
    )?;

    // Every checked level, observed before any of it is compared: the
    // spawn-preflight carve-out relates a worker's ancestor traversal entries
    // to the leaf grant below them, so the expected set cannot be decided one
    // level at a time.
    control.stage("posture-contract");
    let mut observed_acls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for tree_id in CHECKED_TREES {
        let entry = tree(&contract, tree_id)?;
        for level in levels(entry)? {
            let path = level_path(entry, level)?;
            if level_exists(control, &path)? {
                observe_acl(control, &mut observed_acls, &path)?;
            }
        }
    }
    let spawn_entries = spawn_preflight_entries(control, &observed_acls)?;
    for tree_id in CHECKED_TREES {
        let entry = tree(&contract, tree_id)?;
        for level in levels(entry)? {
            check_level(
                control,
                entry,
                level,
                &mut observed_acls,
                &spawn_entries,
                &principal_ids,
            )?;
        }
    }

    // The guest start above ran the daemon's anchored store-view walk while
    // the chain was capped at search-only by the spawn-time `u:d2bd:--x`
    // ACL. Two live proofs: the store-view open never failed, and the resolved
    // view reached a virtiofsd worker through the daemon's fd handoff.
    control.stage("anchor-open-rule");
    control.fail(
        &["journalctl -u d2bd.service --no-pager -b -n 5000 | grep -F 'store-view-open'"],
        None,
    )?;
    if control.execute("pgrep -x virtiofsd >/dev/null", None)?.status != 0 {
        return Err(LegacyError::Assertion(
            "the store-view directory must reach a virtiofsd worker".to_owned(),
        ));
    }
    // Live denial, from the same contract rows: the daemon may search the
    // per-Guest state dir it does not own but may not read it.
    let daemon_read = run_as(
        control,
        &principal_ids,
        "d2bd",
        &format!("ls {} >/dev/null 2>&1", shlex_quote(&guest_state)),
    )?;
    if daemon_read {
        return Err(LegacyError::Assertion(
            "the daemon must not read the per-Guest state dir it only traverses".to_owned(),
        ));
    }

    control.stage("done");
    control.announce("[d2b] declared state posture contract holds on the live host");
    Ok(())
}

/// One declared tree, which must exist exactly once.
fn tree<'a>(contract: &'a Value, tree_id: &str) -> LegacyResult<&'a Value> {
    let matches = contract
        .get("trees")
        .and_then(Value::as_array)
        .map(|trees| {
            trees
                .iter()
                .filter(|entry| entry.get("id").and_then(Value::as_str) == Some(tree_id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if matches.len() != 1 {
        return Err(LegacyError::Assertion(format!(
            "contract tree {tree_id} must exist exactly once"
        )));
    }
    Ok(matches[0])
}

/// One tree's declared levels.
fn levels(entry: &Value) -> LegacyResult<Vec<&Value>> {
    entry
        .get("levels")
        .and_then(Value::as_array)
        .map(|levels| levels.iter().collect())
        .ok_or_else(|| LegacyError::Assertion("a declared tree carries no levels".to_owned()))
}

/// One field of a declaration, as the fixture read it: a missing field is a
/// check failure rather than a lane failure, in the fixture's own words.
fn field<'a>(value: &'a Value, key: &str, where_: &str) -> LegacyResult<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no {key}")))
}

/// The `<token>` substitutions the declaration uses.
fn substitute(value: &str) -> String {
    let tokens = [
        ("state-root", STATE_ROOT),
        ("zone", ZONE),
        ("guest", GUEST),
        ("vm", GUEST),
    ];
    let mut substituted = value.to_owned();
    for (token, replacement) in tokens {
        substituted = substituted.replace(&format!("<{token}>"), replacement);
    }
    substituted
}

/// The live path one declared level names.
fn level_path(entry: &Value, level: &Value) -> LegacyResult<String> {
    let where_ = format!(
        "{}:{}",
        entry.get("id").and_then(Value::as_str).unwrap_or("?"),
        level.get("path").and_then(Value::as_str).unwrap_or("?")
    );
    let root = substitute(
        field(entry, "root", &where_)?
            .as_str()
            .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no root")))?,
    );
    let path = field(level, "path", &where_)?
        .as_str()
        .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no path")))?;
    if path == "." {
        return Ok(root);
    }
    Ok(format!("{root}/{}", substitute(path)))
}

/// A label a level is reported under, the fixture's own `where` text.
fn where_(entry: &Value, level: &Value, path: &str) -> String {
    format!(
        "{}:{} ({path})",
        entry.get("id").and_then(Value::as_str).unwrap_or("?"),
        level.get("path").and_then(Value::as_str).unwrap_or("?"),
    )
}

/// The owner, group and mode of a live path.
fn stat_row(control: &mut GuestControl, path: &str) -> LegacyResult<Stat> {
    let output = control.succeed(&[&format!("stat -c '%U %G %a' {}", shlex_quote(path))], None)?;
    let mut fields = output.split_whitespace();
    match (fields.next(), fields.next(), fields.next(), fields.next()) {
        (Some(owner), Some(group), Some(mode), None) => Ok(Stat {
            owner: owner.to_owned(),
            group: group.to_owned(),
            mode: mode.to_owned(),
        }),
        _ => Err(LegacyError::Assertion(format!(
            "stat reported no owner, group and mode for {path}: {output:?}"
        ))),
    }
}

/// Whether a path exists, without refusing when it does not.
fn level_exists(control: &mut GuestControl, path: &str) -> LegacyResult<bool> {
    Ok(control
        .execute(&format!("test -e {}", shlex_quote(path)), None)?
        .status
        == 0)
}

/// Observe one path's ACL entries, once, and keep them.
///
/// The cache is the fixture's own: the spawn-preflight carve-out is
/// structural, so the whole observed set is read before any of it is judged.
fn observe_acl(
    control: &mut GuestControl,
    observed: &mut BTreeMap<String, BTreeSet<String>>,
    path: &str,
) -> LegacyResult<()> {
    if observed.contains_key(path) {
        return Ok(());
    }
    let output = control.succeed(
        &[&format!("getfacl -cp {} 2>/dev/null || true", shlex_quote(path))],
        None,
    )?;
    let mut entries = BTreeSet::new();
    for line in output.lines() {
        let parts = line.trim().split(':').collect::<Vec<_>>();
        if parts.len() != 3 {
            continue;
        }
        let (kind, name, permissions) = (parts[0], parts[1], parts[2]);
        let short = match kind {
            "user" => Some("u"),
            "group" => Some("g"),
            "other" => Some("o"),
            "mask" => Some("m"),
            _ => None,
        };
        if let Some(short) = short {
            entries.insert(format!("{short}:{name}:{permissions}"));
        }
    }
    observed.insert(path.to_owned(), entries);
    Ok(())
}

/// One ACL entry's permissions, as the set of bits it grants.
fn permission_bits(permissions: &str) -> BTreeSet<char> {
    "rwx".chars().filter(|bit| permissions.contains(*bit)).collect()
}

/// Whether an ACL entry names a principal rather than a class.
fn named_entry(spec: &str) -> bool {
    let parts = spec.splitn(3, ':').collect::<Vec<_>>();
    parts.len() == 3 && (parts[0] == "u" || parts[0] == "g") && !parts[1].is_empty()
}

/// Named entries the spawn preflight is expected to add, per level.
///
/// The broker's spawn preflight opens the ancestor chain above a
/// runner-owned tree with search (`u:<uid>:--x`) and grants the runner its
/// own leaf (`rwx` for a private state tree, `r-x` for a read-only served
/// view root): `runner_tree_acl_targets` in
/// `packages/d2b-broker/src/live_handlers.rs`, reached from
/// `refresh_spawn_runner_acls` / `grant_serving_worker_launch_acls` /
/// `grant_device_worker_launch_acls`. The runner principals are uids minted
/// per Guest at runtime, so the declaration cannot name them; the check
/// derives them from the live worker processes and then allows an undeclared
/// entry only in that structural shape - `--x` on a level that has a deeper
/// grant of the same uid, or the uid's own topmost grant (`--x` when its leaf
/// is outside the checked levels, else the leaf spelling). A foreign uid, or
/// a wider grant on a level above the worker's own leaf, still fails.
fn spawn_preflight_entries(
    control: &mut GuestControl,
    observed: &BTreeMap<String, BTreeSet<String>>,
) -> LegacyResult<BTreeMap<String, BTreeSet<String>>> {
    let processes = control.succeed(&["ps -eo uid=,comm= --no-headers"], None)?;
    let mut worker_uids = BTreeSet::new();
    for line in processes.lines() {
        let trimmed = line.trim();
        let (uid, comm) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
        let comm = comm.trim();
        if comm.starts_with("cloud-hyperviso") || comm.starts_with("virtiofsd") {
            worker_uids.insert(uid.to_owned());
        }
    }
    let mut allowed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for uid in worker_uids {
        let prefix = format!("u:{uid}:");
        let grant_paths = observed
            .iter()
            .filter(|(_, entries)| entries.iter().any(|entry| entry.starts_with(&prefix)))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in &grant_paths {
            let prefix_path = format!("{}/", path.trim_end_matches('/'));
            let deeper = grant_paths
                .iter()
                .any(|other| other != path && other.starts_with(&prefix_path));
            let mut spellings = BTreeSet::new();
            spellings.insert(format!("{prefix}--x"));
            if !deeper {
                spellings.insert(format!("{prefix}rwx"));
                spellings.insert(format!("{prefix}r-x"));
            }
            allowed.entry(path.clone()).or_default().extend(spellings);
        }
    }
    Ok(allowed)
}

/// Run one command as one principal, and whether it succeeded.
fn run_as(
    control: &mut GuestControl,
    principal_ids: &BTreeMap<String, (String, String)>,
    principal: &str,
    command: &str,
) -> LegacyResult<bool> {
    let (uid, gid) = principal_ids.get(principal).ok_or_else(|| {
        LegacyError::Assertion(format!("the check has no ids for principal {principal}"))
    })?;
    let status = control
        .execute(
            &format!(
                "setpriv --reuid={uid} --regid={gid} --init-groups /bin/sh -c {}",
                shlex_quote(command)
            ),
            None,
        )?
        .status;
    Ok(status == 0)
}

/// Whether one principal has one right on one path.
fn probe(
    control: &mut GuestControl,
    principal_ids: &BTreeMap<String, (String, String)>,
    principal: &str,
    right: &str,
    path: &str,
) -> LegacyResult<bool> {
    let flag = match right {
        "traverse" => "x",
        "read" => "r",
        "write" => "w",
        _ => {
            return Err(LegacyError::Assertion(format!(
                "unknown right {right} in the declaration"
            )));
        }
    };
    run_as(
        control,
        principal_ids,
        principal,
        &format!("test -{flag} {}", shlex_quote(path)),
    )
}

/// One declared string field of a level, as the fixture read it.
fn declared_str<'a>(level: &'a Value, key: &str, where_: &str) -> LegacyResult<&'a str> {
    field(level, key, where_)?
        .as_str()
        .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no {key}")))
}

/// Compare one declared level with the live host, in the fixture's own order.
fn check_level(
    control: &mut GuestControl,
    entry: &Value,
    level: &Value,
    observed: &mut BTreeMap<String, BTreeSet<String>>,
    spawn_entries: &BTreeMap<String, BTreeSet<String>>,
    principal_ids: &BTreeMap<String, (String, String)>,
) -> LegacyResult<()> {
    let path = level_path(entry, level)?;
    let where_ = where_(entry, level, &path);
    if !level_exists(control, &path)? {
        if level.get("required").and_then(Value::as_bool) == Some(false) {
            return Ok(());
        }
        return Err(LegacyError::Assertion(format!(
            "declared level is missing: {where_}"
        )));
    }

    let stat = stat_row(control, &path)?;
    let mode = u32::from_str_radix(&stat.mode, 8).map_err(|error| {
        LegacyError::Assertion(format!("the mode at {where_} is not octal: {error}"))
    })?;
    let policy = level
        .get("modePolicy")
        .and_then(Value::as_str)
        .unwrap_or("exact");
    match policy {
        "exact" => {
            let declared_mode = field(level, "mode", &where_)?
                .as_str()
                .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no mode")))?;
            let declared = u32::from_str_radix(declared_mode, 8).map_err(|error| {
                LegacyError::Assertion(format!("the declared mode at {where_} is not octal: {error}"))
            })?;
            if mode != declared & 0o7777 {
                return Err(LegacyError::Assertion(format!(
                    "mode drift at {where_}: declared {declared_mode}, observed {}",
                    stat.mode
                )));
            }
            if stat.owner != declared_str(level, "owner", &where_)? {
                return Err(LegacyError::Assertion(format!(
                    "owner drift at {where_}: declared {}, observed {}",
                    declared_str(level, "owner", &where_)?,
                    stat.owner
                )));
            }
            if stat.group != declared_str(level, "group", &where_)? {
                return Err(LegacyError::Assertion(format!(
                    "group drift at {where_}: declared {}, observed {}",
                    declared_str(level, "group", &where_)?,
                    stat.group
                )));
            }
        }
        "group-traverse-minimum" => {
            if stat.owner != declared_str(level, "owner", &where_)? {
                return Err(LegacyError::Assertion(format!(
                    "owner drift at {where_}: declared {}, observed {}",
                    declared_str(level, "owner", &where_)?,
                    stat.owner
                )));
            }
            if stat.group != declared_str(level, "group", &where_)? {
                return Err(LegacyError::Assertion(format!(
                    "group drift at {where_}: declared {}, observed {}",
                    declared_str(level, "group", &where_)?,
                    stat.group
                )));
            }
            if mode & 0o010 == 0 {
                return Err(LegacyError::Assertion(format!(
                    "group search missing at {where_}"
                )));
            }
            if mode & 0o020 != 0 {
                return Err(LegacyError::Assertion(format!(
                    "group write must never be granted at {where_}"
                )));
            }
        }
        "preserve-existing" => {}
        other => {
            return Err(LegacyError::Assertion(format!(
                "unknown modePolicy {other} at {where_}"
            )));
        }
    }

    observe_acl(control, observed, &path)?;
    let entries = observed
        .get(&path)
        .cloned()
        .ok_or_else(|| LegacyError::Assertion(format!("no ACL entries were read at {where_}")))?;
    let declared_acl = level
        .get("acl")
        .and_then(Value::as_array)
        .map(|acl| {
            acl.iter()
                .filter_map(|declared| {
                    declared
                        .get("spec")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for spec in &declared_acl {
        if !entries.contains(spec) {
            return Err(LegacyError::Assertion(format!(
                "declared ACL missing at {where_}: {spec}"
            )));
        }
    }

    // A declaration pins its named entries completely: an entry nobody
    // declared is drift, not a harmless extra. The only undeclared entries
    // the live host may carry are the spawn-preflight traversal grants derived
    // for the runner uids; the base entries (u::/g::/o::/m::) are pinned only
    // when the declaration names them.
    let declared_named = declared_acl
        .iter()
        .filter(|spec| named_entry(spec))
        .cloned()
        .collect::<BTreeSet<_>>();
    let observed_named = entries
        .iter()
        .filter(|entry| named_entry(entry))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut expected_named = declared_named.clone();
    if let Some(spawned) = spawn_entries.get(&path) {
        expected_named.extend(spawned.iter().cloned());
    }
    let undeclared = observed_named
        .difference(&expected_named)
        .cloned()
        .collect::<Vec<_>>();
    if !undeclared.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "undeclared ACL entry at {where_}: {}",
            undeclared.join(", ")
        )));
    }
    let missing = declared_named
        .difference(&observed_named)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "declared ACL entry missing at {where_}: {}",
            missing.join(", ")
        )));
    }

    // An unpinned mask must be the union of the group entry and the named
    // grants (setfacl semantics): a mask that drifts from that silently
    // re-scopes the whole group class.
    let declared_mask = declared_acl
        .iter()
        .any(|spec| spec.splitn(3, ':').next() == Some("m"));
    let mask_entry = entries.iter().find(|entry| entry.starts_with("m::"));
    if !declared_mask {
        if let Some(mask_entry) = mask_entry {
            let group_entry = entries.iter().find(|entry| entry.starts_with("g::"));
            let mut expected_mask = group_entry
                .map(|entry| permission_bits(entry.splitn(3, ':').nth(2).unwrap_or("")))
                .unwrap_or_default();
            for spec in &observed_named {
                expected_mask.extend(permission_bits(spec.splitn(3, ':').nth(2).unwrap_or("")));
            }
            let observed_mask = permission_bits(mask_entry.splitn(3, ':').nth(2).unwrap_or(""));
            if observed_mask != expected_mask {
                let rendered = ["r", "w", "x"]
                    .into_iter()
                    .filter(|bit| expected_mask.contains(&bit.chars().next().unwrap_or(' ')))
                    .collect::<String>();
                return Err(LegacyError::Assertion(format!(
                    "ACL mask drift at {where_}: expected {rendered} from the group entry and \
                     named grants, observed {mask_entry}"
                )));
            }
        }
    }

    // The declared rights: an expectation of `preserve` or `not-required` is
    // not probed, and root's allow rows are documentation because root
    // bypasses DAC.
    let rights = level
        .get("rights")
        .and_then(Value::as_object)
        .ok_or_else(|| LegacyError::Assertion(format!("{where_} declares no rights")))?;
    for (principal, rights) in rights {
        for right in ["traverse", "read", "write"] {
            let expectation = rights.get(right).and_then(Value::as_str).unwrap_or("preserve");
            if expectation == "preserve" || expectation == "not-required" {
                continue;
            }
            if principal == "root" {
                continue;
            }
            let observed_right = probe(control, principal_ids, principal, right, &path)?;
            if observed_right != (expectation == "allow") {
                return Err(LegacyError::Assertion(format!(
                    "{where_}: {principal} {right} expected {expectation}, observed {}",
                    if observed_right { "allow" } else { "deny" }
                )));
            }
        }
    }

    Ok(())
}

/// One owned row, as the diagnostics rows are passed.
fn as_row<'a>(pair: &'a (String, String)) -> DiagRow<'a> {
    (pair.0.as_str(), pair.1.as_str())
}
