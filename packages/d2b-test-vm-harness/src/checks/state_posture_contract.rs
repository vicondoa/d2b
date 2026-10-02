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
        &[
            as_row(&guest_state_row),
            (
                "live process table",
                "ps -eo pid=,ppid=,args= --no-headers 2>/dev/null | head -n 80 || true",
            ),
            (
                "guest state rows",
                "find /var/lib/d2b/zones/work/guests -maxdepth 3 2>/dev/null | head -n 60 || true",
            ),
        ],
        &[
            ("d2bd.service", ""),
            ("d2b-broker.service", ""),
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

/// The principal spelling `getfacl -cp` renders one uid as.
///
/// [`observe_acl`] reads the kernel's ACL without `--numeric`, so every
/// entry it compares carries its account NAME wherever the uid has a
/// passwd entry and falls back to the number only where it does not. A
/// runner principal minted per Guest at launch had no passwd entry and so
/// rendered numerically, which is why keying the derived set off the
/// numeric `ps -eo uid=` used to line up. Host account provisioning
/// (`tests/unit/nix/cases/host-worker-accounts.json`) gave every runner a
/// named account, so from then on the numeric prefix matched nothing and
/// each named runner's own traversal grant read as undeclared drift.
/// Resolve the spelling the reader will see instead of assuming the
/// numeric one. `id -un` exits non-zero for a uid with no passwd entry, so
/// the `|| true` keeps that case on the numeric spelling rather than
/// refusing the level.
fn acl_principal_spelling(control: &mut GuestControl, uid: &str) -> LegacyResult<String> {
    let name = control
        .succeed(
            &[&format!("id -un {} 2>/dev/null || true", shlex_quote(uid))],
            None,
        )?
        .trim()
        .to_owned();
    Ok(if name.is_empty() {
        uid.to_owned()
    } else {
        name
    })
}

/// The spawn-preflight entries one runner principal's own grants imply, per
/// level.
///
/// The pure half of [`spawn_preflight_entries`]: `grant_paths` are the levels
/// where that principal already holds a grant. Every one of them carries
/// search, and only the principal's own topmost grant (the leaf the
/// preflight opened to it, which may sit outside the checked levels) carries
/// the full leaf spelling: a deeper grant below a path is what makes that
/// path an ancestor, and an ancestor is search-only.
fn spawn_preflight_grants(
    principal: &str,
    grant_paths: &[String],
) -> BTreeMap<String, BTreeSet<String>> {
    let prefix = format!("u:{principal}:");
    let mut allowed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in grant_paths {
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
        allowed
            .entry(path.clone())
            .or_default()
            .extend(spellings);
    }
    allowed
}

/// Named entries the spawn preflight is expected to add, per level.
///
/// The broker's spawn preflight opens the ancestor chain above a
/// runner-owned tree with search (`u:<principal>:--x`) and grants the runner
/// its own leaf (`rwx` for a private state tree, `r-x` for a read-only
/// served view root): `runner_tree_acl_targets` and
/// `served_view_root_acl_targets` in
/// `packages/d2b-broker/src/live_handlers.rs`, reached from
/// `refresh_spawn_runner_acls` / `grant_serving_worker_launch_acls` /
/// `grant_device_worker_launch_acls`. Those principals are named per Zone
/// and per row class, so a host-global declared level cannot name one: the
/// account name itself carries the Zone. The check derives them from the
/// live worker processes instead, spelling each the way [`observe_acl`]
/// read it ([`acl_principal_spelling`]), and allows an undeclared entry
/// only in that structural shape: `--x` on a level that has a deeper grant
/// of the same principal, or that principal's own topmost grant. A foreign
/// principal, an entry on a level that principal holds no grant on, or a
/// wider grant on a level above its own leaf, still fails.
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
        let principal = acl_principal_spelling(control, &uid)?;
        let prefix = format!("u:{principal}:");
        let grant_paths = observed
            .iter()
            .filter(|(_, entries)| entries.iter().any(|entry| entry.starts_with(&prefix)))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for (path, entries) in spawn_preflight_grants(&principal, &grant_paths) {
            allowed.entry(path).or_default().extend(entries);
        }
    }
    Ok(allowed)
}

/// The named ACL entries one level's live ACL carries that neither its
/// declaration nor the derived spawn-preflight grants cover.
///
/// An empty answer is the pass. A declaration pins its named entries
/// completely, so an entry nobody declared is drift rather than a harmless
/// extra; the only undeclared entries the live host may carry are the
/// spawn-preflight grants [`spawn_preflight_grants`] derives for a live
/// runner principal, and the base `u::`/`g::`/`o::`/`m::` rows are pinned
/// only when the declaration names them.
fn undeclared_named_entries(
    observed: &BTreeSet<String>,
    declared: &[String],
    spawn_entries: &BTreeMap<String, BTreeSet<String>>,
    path: &str,
) -> Vec<String> {
    let mut expected = declared
        .iter()
        .filter(|spec| named_entry(spec))
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(spawned) = spawn_entries.get(path) {
        expected.extend(spawned.iter().cloned());
    }
    observed
        .iter()
        .filter(|entry| named_entry(entry))
        .filter(|entry| !expected.contains(*entry))
        .cloned()
        .collect()
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
    // for the live runner principals; the base entries (u::/g::/o::/m::) are
    // pinned only when the declaration names them.
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
    let undeclared = undeclared_named_entries(&observed_named, &declared_acl, spawn_entries, &path);
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
        .any(|spec| spec.split(':').next() == Some("m"));
    let mask_entry = entries.iter().find(|entry| entry.starts_with("m::"));
    if !declared_mask
        && let Some(mask_entry) = mask_entry
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The host state root the serving worker's traversal grant lands on.
    const STATE_ROOT: &str = "/var/lib/d2b";
    /// The served view root the worker is launched to read.
    const VIEW_ROOT: &str =
        "/var/lib/d2b/zones/work/guests/acceptance-guest/store-view/live";
    /// The Zone serving-worker account host account provisioning creates.
    const RUNNER: &str = "d2b-work-virtiofsd";

    /// One observed ACL, as `observe_acl` renders it.
    fn observed(entries: &[&str]) -> BTreeSet<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    /// The spawn-preflight grants every live runner's own grant paths imply,
    /// exactly as `spawn_preflight_entries` derives them from the observed
    /// set: the same walk, over the same levels, for `principal`.
    fn derived(
        observed: &BTreeMap<String, BTreeSet<String>>,
        principal: &str,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let prefix = format!("u:{principal}:");
        let grant_paths = observed
            .iter()
            .filter(|(_, entries)| entries.iter().any(|entry| entry.starts_with(&prefix)))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        spawn_preflight_grants(principal, &grant_paths)
    }

    /// The live shape host account provisioning produced: the daemon's
    /// declared traversal entry plus the Zone serving worker holding search
    /// on the state root and read+traverse on its own view root. Neither
    /// worker entry is named by any declaration - the account name carries
    /// the Zone, so no host-global level can - so the derived grants are the
    /// only thing that can cover them, and they do.
    #[test]
    fn a_named_runner_search_grant_on_the_state_root_is_covered() {
        let live = BTreeMap::from([
            (
                STATE_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2bd:--x",
                    "u:d2b-work-virtiofsd:--x",
                    "g::r-x",
                    "m::r-x",
                    "o::---",
                ]),
            ),
            (
                VIEW_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2b-work-virtiofsd:r-x",
                    "g::r-x",
                    "m::r-x",
                    "o::r-x",
                ]),
            ),
        ]);
        let spawn = derived(&live, RUNNER);
        let declared = vec!["u:d2bd:--x".to_owned()];
        assert!(
            undeclared_named_entries(&live[STATE_ROOT], &declared, &spawn, STATE_ROOT).is_empty(),
            "the worker's own search grant above its view root is the derived shape"
        );
        assert!(
            undeclared_named_entries(&live[VIEW_ROOT], &[], &spawn, VIEW_ROOT).is_empty(),
            "the read-only view root is the worker's own topmost grant"
        );
        assert_eq!(
            spawn[STATE_ROOT],
            observed(&["u:d2b-work-virtiofsd:--x"]),
            "an ancestor of the worker's leaf is search-only"
        );
    }

    /// The gate must still refuse an entry no declaration and no runner
    /// covers. This is the assertion the whole comparison exists for, so
    /// it is pinned here rather than only on the lane.
    #[test]
    fn an_entry_for_a_principal_that_is_no_live_runner_is_undeclared() {
        let live = BTreeMap::from([(
            STATE_ROOT.to_owned(),
            observed(&[
                "u::rwx",
                "u:d2bd:--x",
                "u:d2b-work-virtiofsd:--x",
                "u:d2b-work-virtiofsd-backdoor:--x",
                "g::r-x",
                "m::r-x",
                "o::---",
            ]),
        )]);
        let spawn = derived(&live, RUNNER);
        let declared = vec!["u:d2bd:--x".to_owned()];
        assert_eq!(
            undeclared_named_entries(&live[STATE_ROOT], &declared, &spawn, STATE_ROOT),
            vec!["u:d2b-work-virtiofsd-backdoor:--x".to_owned()],
            "a principal no live worker runs as is drift, however narrow the grant"
        );
    }

    /// The derived set covers the worker's OWN leaf and search on the
    /// levels above it. Read on a level above that leaf is a different and
    /// wider grant than the traversal the preflight makes, so it is refused.
    #[test]
    fn a_wider_grant_above_the_runner_own_leaf_is_undeclared() {
        let live = BTreeMap::from([
            (
                STATE_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2bd:--x",
                    "u:d2b-work-virtiofsd:r-x",
                    "g::r-x",
                    "m::r-x",
                    "o::---",
                ]),
            ),
            (
                VIEW_ROOT.to_owned(),
                observed(&["u::rwx", "u:d2b-work-virtiofsd:rwx", "g::r-x", "m::rwx", "o::---"]),
            ),
        ]);
        let spawn = derived(&live, RUNNER);
        let declared = vec!["u:d2bd:--x".to_owned()];
        assert_eq!(
            undeclared_named_entries(&live[STATE_ROOT], &declared, &spawn, STATE_ROOT),
            vec!["u:d2b-work-virtiofsd:r-x".to_owned()],
            "search above the leaf is the grant; read above it is not"
        );
    }

    /// The other half of the same rule, on the live host's other shape: the
    /// shared runtime root is where the serving worker's private socket tree
    /// begins, so the worker holds a grant there with nothing below it and
    /// that level is its own topmost grant.
    #[test]
    fn a_runner_grant_with_no_deeper_grant_is_the_runner_own_topmost() {
        const RUNTIME_ROOT: &str = "/run/d2b";
        let live = BTreeMap::from([
            (
                RUNTIME_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2bd:rwx",
                    "u:d2b-work-virtiofsd:--x",
                    "g::r-x",
                    "m::rwx",
                    "o::---",
                ]),
            ),
            (
                STATE_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2bd:--x",
                    "u:d2b-work-virtiofsd:--x",
                    "g::r-x",
                    "m::r-x",
                    "o::---",
                ]),
            ),
            (
                VIEW_ROOT.to_owned(),
                observed(&[
                    "u::rwx",
                    "u:d2b-work-virtiofsd:r-x",
                    "g::r-x",
                    "m::r-x",
                    "o::r-x",
                ]),
            ),
        ]);
        let spawn = derived(&live, RUNNER);
        let runtime_declared = vec!["u:d2bd:rwx".to_owned()];
        assert!(
            undeclared_named_entries(
                &live[RUNTIME_ROOT],
                &runtime_declared,
                &spawn,
                RUNTIME_ROOT
            )
            .is_empty(),
            "nothing is granted below the runtime root, so the worker's grant there is its own"
        );
        assert!(
            spawn[RUNTIME_ROOT].contains("u:d2b-work-virtiofsd:rwx"),
            "the topmost grant carries the leaf spellings, not only search"
        );
        let state_declared = vec!["u:d2bd:--x".to_owned()];
        assert!(
            undeclared_named_entries(&live[STATE_ROOT], &state_declared, &spawn, STATE_ROOT)
                .is_empty(),
            "the state root is an ancestor of the view root, so search only"
        );
        assert!(
            undeclared_named_entries(&live[VIEW_ROOT], &[], &spawn, VIEW_ROOT).is_empty(),
            "a read-only attachment's leaf is read+traverse"
        );
    }
}
