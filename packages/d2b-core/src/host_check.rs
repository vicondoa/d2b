use crate::{
    closures::ClosureMetadata,
    host::{HostJson, ModuleRequirement},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCheckReport {
    pub strict: bool,
    pub summary: HostCheckSummary,
    pub findings: Vec<HostCheckFinding>,
}

impl HostCheckReport {
    pub fn exit_code(&self) -> u8 {
        if self.summary.fail > 0 {
            2
        } else if self.summary.warn > 0 {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCheckSummary {
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCheckFinding {
    pub id: String,
    pub severity: HostCheckSeverity,
    pub message: String,
    pub remediation: String,
    pub vm: Option<String>,
    pub detail: Option<String>,
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCheckSeverity {
    Pass,
    Warn,
    Fail,
}

impl HostCheckSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeError {
    pub opaque_reason: String,
}

impl ProbeError {
    pub fn new(opaque_reason: impl Into<String>) -> Self {
        Self {
            opaque_reason: opaque_reason.into(),
        }
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.opaque_reason)
    }
}

impl std::error::Error for ProbeError {}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct HostCheckFixture {
    kernel_release: Option<String>,
    cgroup_v2_present: Option<bool>,
    cpu_vendor: Option<String>,
    loaded_modules: Option<Vec<String>>,
    built_in_modules: Option<Vec<String>>,
    loaded_modules_error: Option<String>,
    nft_has_d2b_table: Option<bool>,
    nft_error: Option<String>,
    firewalld_active: Option<bool>,
    firewalld_error: Option<String>,
    ufw_active: Option<bool>,
    ufw_error: Option<String>,
    systemctl_unavailable: Option<bool>,
    sysctls: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModulePresence {
    Loaded,
    BuiltIn,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceProbeState {
    Active,
    Inactive,
    Unavailable,
}

impl ServiceProbeState {
    const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Unavailable => "probe-unavailable",
        }
    }

    const fn as_bool(self) -> Option<bool> {
        match self {
            Self::Active => Some(true),
            Self::Inactive => Some(false),
            Self::Unavailable => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ServiceProbeKind {
    Firewalld,
    Ufw,
}

impl ServiceProbeKind {
    const fn unit(self) -> &'static str {
        match self {
            Self::Firewalld => "firewalld.service",
            Self::Ufw => "ufw.service",
        }
    }

    const fn active_env(self) -> &'static str {
        match self {
            Self::Firewalld => "D2B_TEST_FIREWALLD_ACTIVE",
            Self::Ufw => "D2B_TEST_UFW_ACTIVE",
        }
    }

    const fn probe_context(self) -> &'static str {
        match self {
            Self::Firewalld => "run systemctl is-active firewalld.service",
            Self::Ufw => "run systemctl is-active ufw.service",
        }
    }

    fn fixture_error(self, fixture: &HostCheckFixture) -> Option<String> {
        match self {
            Self::Firewalld => fixture.firewalld_error.clone(),
            Self::Ufw => fixture.ufw_error.clone(),
        }
    }

    fn fixture_active(self, fixture: &HostCheckFixture) -> Option<bool> {
        match self {
            Self::Firewalld => fixture.firewalld_active,
            Self::Ufw => fixture.ufw_active,
        }
    }
}

struct ProbeSource {
    fixture: Option<HostCheckFixture>,
}

impl ProbeSource {
    fn from_env() -> Result<Self, ProbeError> {
        let fixture = match env::var_os("D2B_HOST_CHECK_FIXTURE") {
            Some(path) => {
                let path = PathBuf::from(path);
                let bytes = fs::read(&path).map_err(|err| {
                    probe_error(format!("read fixture {}", path.display()), err.to_string())
                })?;
                let fixture = serde_json::from_slice(&bytes).map_err(|err| {
                    probe_error(
                        format!("decode fixture {}", path.display()),
                        err.to_string(),
                    )
                })?;
                Some(fixture)
            }
            None => None,
        };
        Ok(Self { fixture })
    }

    fn kernel_release(&self) -> Result<String, ProbeError> {
        if let Some(value) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.kernel_release.clone())
        {
            return Ok(value);
        }
        read_trimmed(
            Path::new("/proc/sys/kernel/osrelease"),
            "read /proc/sys/kernel/osrelease",
        )
    }

    fn cgroup_v2_present(&self) -> Result<bool, ProbeError> {
        if let Some(value) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.cgroup_v2_present)
        {
            return Ok(value);
        }
        path_exists(
            Path::new("/sys/fs/cgroup/cgroup.controllers"),
            "stat /sys/fs/cgroup/cgroup.controllers",
        )
    }

    fn cpu_vendor(&self) -> Result<String, ProbeError> {
        if let Some(value) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.cpu_vendor.clone())
        {
            return Ok(value);
        }
        let cpuinfo = fs::read_to_string("/proc/cpuinfo")
            .map_err(|err| probe_error("read /proc/cpuinfo", err.to_string()))?;
        Ok(cpuinfo
            .lines()
            .find(|line| line.starts_with("vendor_id"))
            .and_then(|line| line.split(':').nth(1))
            .map(str::trim)
            .map(|vendor| match vendor {
                "GenuineIntel" => "intel",
                "AuthenticAMD" => "amd",
                _ => "unknown",
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "unknown".to_owned()))
    }

    fn loaded_modules(&self) -> Result<BTreeSet<String>, ProbeError> {
        if let Some(detail) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.loaded_modules_error.clone())
        {
            return Err(probe_error("read /proc/modules", detail));
        }
        if let Some(modules) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.loaded_modules.clone())
        {
            return Ok(modules.into_iter().collect());
        }
        let contents = fs::read_to_string("/proc/modules")
            .map_err(|err| probe_error("read /proc/modules", err.to_string()))?;
        Ok(contents
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .collect())
    }

    fn module_presence(
        &self,
        module: &str,
        loaded_modules: &BTreeSet<String>,
    ) -> Result<ModulePresence, ProbeError> {
        if loaded_modules.contains(module) {
            return Ok(ModulePresence::Loaded);
        }
        if let Some(fixture) = &self.fixture {
            let built_in = fixture
                .built_in_modules
                .as_ref()
                .map(|modules| modules.iter().any(|candidate| candidate == module))
                .unwrap_or(false);
            return Ok(if built_in {
                ModulePresence::BuiltIn
            } else {
                ModulePresence::Absent
            });
        }
        if path_exists(
            &PathBuf::from("/sys/module").join(module),
            &format!("stat /sys/module/{module}"),
        )? {
            Ok(ModulePresence::BuiltIn)
        } else {
            Ok(ModulePresence::Absent)
        }
    }

    fn nft_has_d2b_table(&self) -> Result<bool, ProbeError> {
        if let Some(detail) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.nft_error.clone())
        {
            return Err(probe_error("run nft list ruleset --json", detail));
        }
        if let Some(value) = self
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.nft_has_d2b_table)
        {
            return Ok(value);
        }
        let output = Command::new("nft")
            .args(["list", "ruleset", "--json"])
            .env_remove("NOTIFY_SOCKET")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|err| probe_error("run nft list ruleset --json", err.to_string()))?;
        if !output.status.success() {
            return Err(probe_error(
                "run nft list ruleset --json",
                format!("status {} ({})", output.status, output_text(&output)),
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|err| probe_error("parse nft list ruleset --json", err.to_string()))?;
        Ok(value
            .get("nftables")
            .and_then(Value::as_array)
            .map(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .get("table")
                        .map(|table| {
                            table.get("family").and_then(Value::as_str) == Some("inet")
                                && table.get("name").and_then(Value::as_str) == Some("d2b")
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false))
    }

    fn service_state(&self, kind: ServiceProbeKind) -> Result<ServiceProbeState, ProbeError> {
        if let Some(detail) = self
            .fixture
            .as_ref()
            .and_then(|fixture| kind.fixture_error(fixture))
        {
            return Err(probe_error(kind.probe_context(), detail));
        }
        if let Some(value) = env_bool(kind.active_env()) {
            return Ok(if value {
                ServiceProbeState::Active
            } else {
                ServiceProbeState::Inactive
            });
        }
        if env_bool("D2B_TEST_SYSTEMCTL_UNAVAILABLE").unwrap_or(false)
            || self
                .fixture
                .as_ref()
                .and_then(|fixture| fixture.systemctl_unavailable)
                .unwrap_or(false)
        {
            return Ok(ServiceProbeState::Unavailable);
        }
        if let Some(value) = self
            .fixture
            .as_ref()
            .and_then(|fixture| kind.fixture_active(fixture))
        {
            return Ok(if value {
                ServiceProbeState::Active
            } else {
                ServiceProbeState::Inactive
            });
        }
        let output = Command::new("systemctl")
            .args(["is-active", kind.unit()])
            .env_remove("NOTIFY_SOCKET")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(output) if output.status.success() => Ok(ServiceProbeState::Active),
            Ok(output) if systemctl_probe_unavailable(&output) => {
                Ok(ServiceProbeState::Unavailable)
            }
            Ok(_) => Ok(ServiceProbeState::Inactive),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(ServiceProbeState::Unavailable),
            Err(err) => Err(probe_error(kind.probe_context(), err.to_string())),
        }
    }

    fn sysctl_value(&self, if_name: &str, field: &str) -> Result<Option<String>, ProbeError> {
        let key = format!("{if_name}.{field}");
        if let Some(fixture) = &self.fixture {
            return Ok(fixture.sysctls.get(&key).cloned());
        }
        match fs::read_to_string(format!("/proc/sys/net/ipv6/conf/{if_name}/{field}")) {
            Ok(value) => Ok(Some(value.trim().to_owned())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(probe_error(
                format!("read /proc/sys/net/ipv6/conf/{if_name}/{field}"),
                err.to_string(),
            )),
        }
    }

    /// Read an arbitrary `/proc/sys` key written in dotted form
    /// (e.g. `net.bridge.bridge-nf-call-iptables`). Used to enforce
    /// `HostJson.kernel_modules[].sysctls` when a module is loaded/built-in.
    /// Fixture mode looks up the
    /// dotted key directly in `HostCheckFixture.sysctls`.
    fn module_sysctl_value(&self, dotted_key: &str) -> Result<Option<String>, ProbeError> {
        if let Some(fixture) = &self.fixture {
            return Ok(fixture.sysctls.get(dotted_key).cloned());
        }
        let path: PathBuf = std::iter::once("/proc/sys")
            .chain(dotted_key.split('.'))
            .collect();
        match fs::read_to_string(&path) {
            Ok(value) => Ok(Some(value.trim().to_owned())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(probe_error(
                format!("read {}", path.display()),
                err.to_string(),
            )),
        }
    }
}

pub fn run<'a, I>(host: &HostJson, closures: I, strict: bool) -> Result<HostCheckReport, ProbeError>
where
    I: IntoIterator<Item = &'a ClosureMetadata>,
{
    let probe = ProbeSource::from_env()?;
    run_with_probe(host, &probe, closures, strict)
}

#[cfg(test)]
pub(crate) fn run_with_fixture<'a, I>(
    host: &HostJson,
    fixture: HostCheckFixture,
    closures: I,
    strict: bool,
) -> Result<HostCheckReport, ProbeError>
where
    I: IntoIterator<Item = &'a ClosureMetadata>,
{
    let probe = ProbeSource {
        fixture: Some(fixture),
    };
    run_with_probe(host, &probe, closures, strict)
}

fn run_with_probe<'a, I>(
    host: &HostJson,
    probe: &ProbeSource,
    closures: I,
    strict: bool,
) -> Result<HostCheckReport, ProbeError>
where
    I: IntoIterator<Item = &'a ClosureMetadata>,
{
    let kernel_release = probe.kernel_release()?;
    let cgroup_v2_present = probe.cgroup_v2_present()?;
    let cpu_vendor = probe.cpu_vendor()?;
    let loaded_modules = probe.loaded_modules()?;
    let nft_has_d2b_table = probe.nft_has_d2b_table()?;
    let firewalld_state = probe.service_state(ServiceProbeKind::Firewalld)?;
    let ufw_state = probe.service_state(ServiceProbeKind::Ufw)?;

    let mut findings = Vec::new();

    findings.push(if kernel_version_ok(&kernel_release) {
        finding(
            "kernel-version",
            HostCheckSeverity::Pass,
            format!("kernel {} satisfies the >= 6.6 requirement", kernel_release),
            "Keep a 6.6+ kernel available for daemon experiments.",
        )
    } else {
        finding(
            "kernel-version",
            HostCheckSeverity::Fail,
            format!(
                "kernel {} is older than the required 6.6 baseline",
                kernel_release
            ),
            "Upgrade the host kernel to >= 6.6 before enabling daemon-experimental mode.",
        )
    });

    findings.push(if cgroup_v2_present {
        finding(
            "cgroup-v2",
            HostCheckSeverity::Pass,
            "/sys/fs/cgroup/cgroup.controllers is present".to_owned(),
            "Keep the unified cgroup-v2 hierarchy enabled.",
        )
    } else {
        finding(
            "cgroup-v2",
            HostCheckSeverity::Fail,
            "the unified cgroup-v2 hierarchy is missing".to_owned(),
            "Boot with unified cgroup-v2 enabled before using the Rust control plane.",
        )
    });

    findings.push(match cpu_vendor.as_str() {
        "intel" | "amd" => finding(
            "cpu-vendor",
            HostCheckSeverity::Pass,
            format!("detected supported CPU vendor `{}`", cpu_vendor),
            "No action required.",
        ),
        other => finding(
            "cpu-vendor",
            HostCheckSeverity::Warn,
            format!("detected unsupported or unknown CPU vendor `{}`", other),
            "Confirm the host can load the correct KVM module before enabling daemon-experimental mode.",
        ),
    });

    for module in &host.kernel_modules {
        let mut details = BTreeMap::new();
        details.insert("gate".to_owned(), module.gate.clone());
        let presence = if matches!(module.requirement, ModuleRequirement::Deferred) {
            ModulePresence::Absent
        } else {
            probe.module_presence(&module.module, &loaded_modules)?
        };
        let (severity, message, remediation) = match module.requirement {
            ModuleRequirement::Required => match presence {
                ModulePresence::Loaded => (
                    HostCheckSeverity::Pass,
                    format!("kernel module `{}` is loaded", module.module),
                    "No action required.",
                ),
                ModulePresence::BuiltIn => (
                    HostCheckSeverity::Pass,
                    format!(
                        "kernel module `{}` is built into the running kernel",
                        module.module
                    ),
                    "No action required.",
                ),
                ModulePresence::Absent => (
                    HostCheckSeverity::Fail,
                    format!("required kernel module `{}` is not present", module.module),
                    "Load the module (or configure it declaratively) before enabling daemon-experimental mode.",
                ),
            },
            ModuleRequirement::Alternatives => {
                let gate_matches = module.gate == format!("host-cpu-vendor={cpu_vendor}");
                if !gate_matches {
                    (
                        HostCheckSeverity::Pass,
                        format!(
                            "kernel module `{}` is not required on this host",
                            module.module
                        ),
                        "No action required.",
                    )
                } else {
                    match presence {
                        ModulePresence::Loaded => (
                            HostCheckSeverity::Pass,
                            format!("vendor-gated kernel module `{}` is loaded", module.module),
                            "No action required.",
                        ),
                        ModulePresence::BuiltIn => (
                            HostCheckSeverity::Pass,
                            format!(
                                "vendor-gated kernel module `{}` is built into the running kernel",
                                module.module
                            ),
                            "No action required.",
                        ),
                        ModulePresence::Absent => (
                            HostCheckSeverity::Fail,
                            format!("vendor-gated kernel module `{}` is missing", module.module),
                            "Load the vendor-specific KVM module required by the host CPU.",
                        ),
                    }
                }
            }
            ModuleRequirement::Optional => match presence {
                ModulePresence::Loaded => (
                    HostCheckSeverity::Pass,
                    format!("optional kernel module `{}` is loaded", module.module),
                    "No action required.",
                ),
                ModulePresence::BuiltIn => (
                    HostCheckSeverity::Pass,
                    format!(
                        "optional kernel module `{}` is built into the running kernel",
                        module.module
                    ),
                    "No action required.",
                ),
                ModulePresence::Absent => (
                    HostCheckSeverity::Warn,
                    format!("optional kernel module `{}` is not present", module.module),
                    "Load the module if you need the feature it enables.",
                ),
            },
            ModuleRequirement::Deferred => (
                HostCheckSeverity::Pass,
                format!(
                    "kernel module `{}` remains deferred to a later wave",
                    module.module
                ),
                "No action required.",
            ),
        };
        findings.push(HostCheckFinding {
            id: format!("kernel-module:{}", module.module),
            severity,
            message,
            remediation: remediation.to_owned(),
            vm: None,
            detail: Some(module.feature.clone()),
            details,
        });

        // When a module is loaded or built-in, enforce its declared
        // `sysctls`. The HostJson
        // entries are `key=value` strings (e.g.
        // `net.bridge.bridge-nf-call-iptables=0`) emitted by the Nix
        // module. For br_netfilter this catches the documented
        // fail-closed guard against bridge traffic traversing legacy
        // iptables/ip6tables/arptables outside the inet d2b
        // policy. Deferred modules and absent optional/alternatives
        // modules skip enforcement (there's no live sysctl to read).
        let sysctls_apply = matches!(presence, ModulePresence::Loaded | ModulePresence::BuiltIn);
        if sysctls_apply {
            for entry in &module.sysctls {
                let (key, expected) = match entry.split_once('=') {
                    Some((key, value)) => (key.trim(), value.trim()),
                    None => {
                        findings.push(HostCheckFinding {
                            id: format!("kernel-module-sysctl:{}:{}", module.module, entry),
                            severity: HostCheckSeverity::Fail,
                            message: format!(
                                "host.json kernelModules[{}].sysctls entry `{}` is not in `key=value` form",
                                module.module, entry
                            ),
                            remediation: "Emit each kernelModules[].sysctls entry as `<dotted.key>=<value>`.".to_owned(),
                            vm: None,
                            detail: Some(module.feature.clone()),
                            details: BTreeMap::from([
                                ("module".to_owned(), module.module.clone()),
                                ("sysctl-entry".to_owned(), entry.clone()),
                            ]),
                        });
                        continue;
                    }
                };
                let observed = probe.module_sysctl_value(key)?;
                let mut details = BTreeMap::new();
                details.insert("module".to_owned(), module.module.clone());
                details.insert("sysctl".to_owned(), key.to_owned());
                details.insert("expected".to_owned(), expected.to_owned());
                let (severity, message, remediation): (HostCheckSeverity, String, String) =
                    match observed.as_deref() {
                        Some(actual) if actual == expected => (
                            HostCheckSeverity::Pass,
                            format!(
                                "kernel module `{}` sysctl `{}` matches the declared value `{}`",
                                module.module, key, expected
                            ),
                            "No action required.".to_owned(),
                        ),
                        Some(actual) => {
                            details.insert("observed".to_owned(), actual.to_owned());
                            (
                                HostCheckSeverity::Fail,
                                format!(
                                    "kernel module `{}` sysctl `{}` drifted from `{}` to `{}`",
                                    module.module, key, expected, actual
                                ),
                                format!(
                                    "Set `{}` back to `{}` (host.json declares this value for module `{}`).",
                                    key, expected, module.module
                                ),
                            )
                        }
                        None => (
                            HostCheckSeverity::Fail,
                            format!(
                                "kernel module `{}` sysctl `{}` could not be read",
                                module.module, key
                            ),
                            format!(
                                "Ensure `/proc/sys/{}` is present and set to `{}` (host.json declares this value for module `{}`).",
                                key.replace('.', "/"),
                                expected,
                                module.module
                            ),
                        ),
                    };
                findings.push(HostCheckFinding {
                    id: format!("kernel-module-sysctl:{}:{}", module.module, key),
                    severity,
                    message,
                    remediation,
                    vm: None,
                    detail: Some(module.feature.clone()),
                    details,
                });
            }
        }
    }

    findings.push(if nft_has_d2b_table {
        finding(
            "nftables-table",
            HostCheckSeverity::Pass,
            "`nft list ruleset --json` shows an `inet d2b` table".to_owned(),
            "No action required.",
        )
    } else {
        finding(
            "nftables-table",
            HostCheckSeverity::Warn,
            "`nft list ruleset --json` did not show an `inet d2b` table".to_owned(),
            "Apply host networking preparation (`d2b host prepare --apply`) when that command lands.",
        )
    });

    findings.push(if matches!(firewalld_state, ServiceProbeState::Inactive)
        && matches!(ufw_state, ServiceProbeState::Inactive)
    {
        finding(
            "firewall-coexistence",
            HostCheckSeverity::Pass,
            "firewalld and ufw are both inactive".to_owned(),
            "No action required.",
        )
    } else if matches!(firewalld_state, ServiceProbeState::Unavailable)
        || matches!(ufw_state, ServiceProbeState::Unavailable)
    {
        let mut details = BTreeMap::new();
        details.insert("firewalld".to_owned(), firewalld_state.label().to_owned());
        details.insert("ufw".to_owned(), ufw_state.label().to_owned());
        HostCheckFinding {
            id: "firewall-coexistence".to_owned(),
            severity: HostCheckSeverity::Warn,
            message: format!(
                "firewall service activity could not be fully determined (firewalld={}, ufw={})",
                firewalld_state.label(),
                ufw_state.label()
            ),
            remediation: "If the host is systemd-based, verify firewalld.service and ufw.service manually before enabling daemon-experimental mode.".to_owned(),
            vm: None,
            detail: Some("systemctl probe unavailable on this host".to_owned()),
            details,
        }
    } else {
        let mut details = BTreeMap::new();
        details.insert("firewalld".to_owned(), firewalld_state.label().to_owned());
        details.insert("ufw".to_owned(), ufw_state.label().to_owned());
        HostCheckFinding {
            id: "firewall-coexistence".to_owned(),
            severity: HostCheckSeverity::Warn,
            message: format!(
                "firewalld_active={} ufw_active={}",
                firewalld_state.as_bool().unwrap_or(false),
                ufw_state.as_bool().unwrap_or(false)
            ),
            remediation: "Disable firewalld/ufw or validate coexistence before enabling daemon-experimental mode.".to_owned(),
            vm: None,
            detail: None,
            details,
        }
    });

    for env in &host.environments {
        for sysctl in &env.ipv6_sysctls {
            for (field, expected) in [
                ("disable_ipv6", sysctl.disable_ipv6.to_string()),
                ("accept_ra", sysctl.accept_ra.to_string()),
                ("autoconf", sysctl.autoconf.to_string()),
                ("addr_gen_mode", sysctl.addr_gen_mode.to_string()),
                ("arp_ignore", sysctl.arp_ignore.to_string()),
            ] {
                let key = format!("{}.{}", sysctl.if_name.as_str(), field);
                let observed = probe.sysctl_value(sysctl.if_name.as_str(), field)?;
                let severity = if observed.as_deref() == Some(expected.as_str()) {
                    HostCheckSeverity::Pass
                } else {
                    HostCheckSeverity::Fail
                };
                let mut details = BTreeMap::new();
                details.insert("expected".to_owned(), expected.clone());
                if let Some(observed) = &observed {
                    details.insert("observed".to_owned(), observed.clone());
                }
                findings.push(HostCheckFinding {
                    id: format!("ipv6-sysctl:{}:{}", env.env, key),
                    severity,
                    message: match &observed {
                        Some(observed) if *observed == expected => {
                            format!("IPv6 sysctl `{}` matches the declared value `{}`", key, expected)
                        }
                        Some(observed) => format!(
                            "IPv6 sysctl `{}` drifted from `{}` to `{}`",
                            key, expected, observed
                        ),
                        None => format!("IPv6 sysctl `{}` could not be read", key),
                    },
                    remediation: "Ensure the declared d2b IPv6 sysctls are applied to every env bridge and tap.".to_owned(),
                    vm: None,
                    detail: None,
                    details,
                });
            }
        }
    }

    for closure in closures {
        let severity = if closure.runner_parity_ok {
            HostCheckSeverity::Pass
        } else if strict {
            HostCheckSeverity::Fail
        } else {
            HostCheckSeverity::Warn
        };
        let mut details = BTreeMap::new();
        details.insert("declaredRunner".to_owned(), closure.declared_runner.clone());
        details.insert(
            "runnerParityPath".to_owned(),
            closure.runner_parity_path.clone(),
        );
        findings.push(HostCheckFinding {
            id: "runner-parity".to_owned(),
            severity,
            message: if closure.runner_parity_ok {
                format!("runner parity matches for `{}`", closure.vm)
            } else {
                format!("runner parity drift detected for `{}`", closure.vm)
            },
            remediation: "Rebuild the VM closure or correct the declared runner path before relying on daemon-experimental mode.".to_owned(),
            vm: Some(closure.vm.clone()),
            detail: Some(if strict {
                "strict mode upgrades runner parity drift from warning to failure".to_owned()
            } else {
                "runner parity drift remains advisory until `--strict` is supplied".to_owned()
            }),
            details,
        });
    }

    Ok(HostCheckReport {
        strict,
        summary: summarize_findings(&findings),
        findings,
    })
}

pub fn kernel_version_ok(release: &str) -> bool {
    let mut parts = release
        .split(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .find(|part| part.contains('.'))
        .unwrap_or(release)
        .split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    major > 6 || (major == 6 && minor >= 6)
}

fn summarize_findings(findings: &[HostCheckFinding]) -> HostCheckSummary {
    let mut summary = HostCheckSummary {
        pass: 0,
        warn: 0,
        fail: 0,
    };
    for finding in findings {
        match finding.severity {
            HostCheckSeverity::Pass => summary.pass += 1,
            HostCheckSeverity::Warn => summary.warn += 1,
            HostCheckSeverity::Fail => summary.fail += 1,
        }
    }
    summary
}

fn finding(
    id: impl Into<String>,
    severity: HostCheckSeverity,
    message: String,
    remediation: &'static str,
) -> HostCheckFinding {
    HostCheckFinding {
        id: id.into(),
        severity,
        message,
        remediation: remediation.to_owned(),
        vm: None,
        detail: None,
        details: BTreeMap::new(),
    }
}

fn read_trimmed(path: &Path, step: &'static str) -> Result<String, ProbeError> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|err| probe_error(step, err.to_string()))
}

fn path_exists(path: &Path, step: &str) -> Result<bool, ProbeError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(probe_error(step, err.to_string())),
    }
}

fn output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout={stdout}; stderr={stderr}"),
        (false, true) => format!("stdout={stdout}"),
        (true, false) => format!("stderr={stderr}"),
        (true, true) => "no output".to_owned(),
    }
}

fn systemctl_probe_unavailable(output: &Output) -> bool {
    let text = output_text(output).to_ascii_lowercase();
    text.contains("system has not been booted with systemd")
        || text.contains("failed to connect to bus")
        || text.contains("host is down")
}

fn env_bool(name: &str) -> Option<bool> {
    env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

fn probe_error(step: impl Into<String>, detail: impl Into<String>) -> ProbeError {
    ProbeError::new(format!("{}: {}", step.into(), detail.into()))
}

#[cfg(test)]
mod module_sysctl_tests {
    //! Coverage for `kernel_modules[].sysctls` enforcement in
    //! [`host_check::run`]. Each test constructs a HostJson with a
    //! single `br_netfilter` module entry that declares the three
    //! documented bridge-nf-call sysctls, then runs `host_check`
    //! against a fixture that varies what the simulated `/proc/sys`
    //! reads return. The new findings carry id form
    //! `kernel-module-sysctl:<module>:<dotted.key>`.

    use std::collections::BTreeMap;

    use crate::host::HostJson;
    use crate::host_check::{HostCheckFixture, HostCheckSeverity, run_with_fixture};

    const BASE_HOST_JSON: &str =
        include_str!("../../../tests/fixtures/deny-unknown/host-valid.json");

    fn br_netfilter_host(loaded: bool) -> HostJson {
        let mut host: HostJson = serde_json::from_str(BASE_HOST_JSON)
            .expect("base host-valid.json fixture must deserialize");
        host.kernel_modules.clear();
        host.kernel_modules
            .push(crate::host::KernelModulesEntry {
                module: "br_netfilter".to_owned(),
                feature: "bridge-netfilter".to_owned(),
                requirement: crate::host::ModuleRequirement::Optional,
                gate: "Optional, but if present d2b fails closed unless all bridge-nf-call sysctls are zero.".to_owned(),
                sysctls: vec![
                    "net.bridge.bridge-nf-call-iptables=0".to_owned(),
                    "net.bridge.bridge-nf-call-ip6tables=0".to_owned(),
                    "net.bridge.bridge-nf-call-arptables=0".to_owned(),
                ],
                jail_visible_device: false,
            });
        let loaded_modules = if loaded {
            vec!["br_netfilter".to_owned()]
        } else {
            vec![]
        };
        let _ = loaded; // silence if-unused on the bind above
        let _ = &loaded_modules;
        host
    }

    fn baseline_fixture(loaded_br_netfilter: bool) -> HostCheckFixture {
        let mut fixture = HostCheckFixture {
            kernel_release: Some("6.8.12-d2b".to_owned()),
            cgroup_v2_present: Some(true),
            cpu_vendor: Some("intel".to_owned()),
            loaded_modules: Some(if loaded_br_netfilter {
                vec!["br_netfilter".to_owned()]
            } else {
                vec![]
            }),
            built_in_modules: None,
            loaded_modules_error: None,
            nft_has_d2b_table: Some(true),
            nft_error: None,
            firewalld_active: Some(false),
            firewalld_error: None,
            ufw_active: Some(false),
            ufw_error: None,
            systemctl_unavailable: None,
            sysctls: BTreeMap::new(),
        };
        // The base HostJson fixture references IPv6 sysctls per env,
        // so the existing per-env IPv6 finding loop also reads from
        // `fixture.sysctls`. Preload safe defaults for every
        // ipv6_sysctls field so those findings stay Pass and don't
        // mask the kernel-module-sysctl findings we're asserting.
        let host = br_netfilter_host(loaded_br_netfilter);
        for env in &host.environments {
            for entry in &env.ipv6_sysctls {
                for (field, value) in [
                    ("disable_ipv6", entry.disable_ipv6.to_string()),
                    ("accept_ra", entry.accept_ra.to_string()),
                    ("autoconf", entry.autoconf.to_string()),
                    ("addr_gen_mode", entry.addr_gen_mode.to_string()),
                    ("arp_ignore", entry.arp_ignore.to_string()),
                ] {
                    fixture
                        .sysctls
                        .insert(format!("{}.{}", entry.if_name.as_str(), field), value);
                }
            }
        }
        fixture
    }

    fn find_finding<'a>(
        report: &'a crate::host_check::HostCheckReport,
        id: &str,
    ) -> &'a crate::host_check::HostCheckFinding {
        report
            .findings
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| {
                panic!(
                    "missing finding id {:?}; ids present: {:?}",
                    id,
                    report.findings.iter().map(|f| &f.id).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn br_netfilter_sysctls_pass_when_all_zero() {
        let mut fixture = baseline_fixture(true);
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-iptables".to_owned(),
            "0".to_owned(),
        );
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-ip6tables".to_owned(),
            "0".to_owned(),
        );
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-arptables".to_owned(),
            "0".to_owned(),
        );

        let host = br_netfilter_host(true);
        let report = run_with_fixture(&host, fixture, std::iter::empty(), false)
            .expect("host_check::run_with_fixture must succeed");

        for sysctl in [
            "net.bridge.bridge-nf-call-iptables",
            "net.bridge.bridge-nf-call-ip6tables",
            "net.bridge.bridge-nf-call-arptables",
        ] {
            let id = format!("kernel-module-sysctl:br_netfilter:{sysctl}");
            let finding = find_finding(&report, &id);
            assert_eq!(
                finding.severity,
                HostCheckSeverity::Pass,
                "{id} must Pass when sysctl is `0`; got {finding:?}",
            );
        }
    }

    #[test]
    fn br_netfilter_sysctl_drift_fails_closed() {
        let mut fixture = baseline_fixture(true);
        // iptables drifted to 1, the other two correct.
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-iptables".to_owned(),
            "1".to_owned(),
        );
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-ip6tables".to_owned(),
            "0".to_owned(),
        );
        fixture.sysctls.insert(
            "net.bridge.bridge-nf-call-arptables".to_owned(),
            "0".to_owned(),
        );

        let host = br_netfilter_host(true);
        let report = run_with_fixture(&host, fixture, std::iter::empty(), false)
            .expect("host_check::run_with_fixture must succeed");

        let bad = find_finding(
            &report,
            "kernel-module-sysctl:br_netfilter:net.bridge.bridge-nf-call-iptables",
        );
        assert_eq!(
            bad.severity,
            HostCheckSeverity::Fail,
            "drifted iptables sysctl must Fail (not Warn / Pass): {bad:?}"
        );
        assert!(
            bad.message.contains("drifted"),
            "drift message must explain drift: {}",
            bad.message
        );

        let ok = find_finding(
            &report,
            "kernel-module-sysctl:br_netfilter:net.bridge.bridge-nf-call-ip6tables",
        );
        assert_eq!(ok.severity, HostCheckSeverity::Pass);
    }

    #[test]
    fn br_netfilter_sysctl_missing_fails_closed() {
        // br_netfilter loaded but /proc/sys returned None (missing
        // sysctl). The previous behavior silently skipped enforcement;
        // this makes it fail closed — operators must explicitly set the
        // documented value.
        let fixture = baseline_fixture(true);
        let host = br_netfilter_host(true);
        let report = run_with_fixture(&host, fixture, std::iter::empty(), false)
            .expect("host_check::run_with_fixture must succeed");

        for sysctl in [
            "net.bridge.bridge-nf-call-iptables",
            "net.bridge.bridge-nf-call-ip6tables",
            "net.bridge.bridge-nf-call-arptables",
        ] {
            let id = format!("kernel-module-sysctl:br_netfilter:{sysctl}");
            let finding = find_finding(&report, &id);
            assert_eq!(
                finding.severity,
                HostCheckSeverity::Fail,
                "{id} must Fail when sysctl is unreadable; got {finding:?}",
            );
            assert!(
                finding.message.contains("could not be read"),
                "missing-sysctl message must say so: {}",
                finding.message
            );
        }
    }

    #[test]
    fn br_netfilter_sysctls_skipped_when_module_absent() {
        // br_netfilter optional + absent → no module-sysctl findings
        // because there is no live sysctl to read.
        let fixture = baseline_fixture(false);
        let host = br_netfilter_host(false);
        let report = run_with_fixture(&host, fixture, std::iter::empty(), false)
            .expect("host_check::run_with_fixture must succeed");

        let any_sysctl_finding = report
            .findings
            .iter()
            .any(|f| f.id.starts_with("kernel-module-sysctl:"));
        assert!(
            !any_sysctl_finding,
            "module-sysctl enforcement must skip absent modules; got: {:?}",
            report
                .findings
                .iter()
                .filter(|f| f.id.starts_with("kernel-module-sysctl:"))
                .collect::<Vec<_>>()
        );
    }
}
