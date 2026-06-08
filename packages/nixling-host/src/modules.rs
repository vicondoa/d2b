//! Kernel-module matrix and probe order.
//!
//! Implements the four-step kernel-module probe order:
//!
//! 1. `/proc/sys/kernel/modules_disabled` — if `1`, every `required`
//!    module that is neither built-in nor loaded forces a closed-fail
//!    `host-modules-locked` finding.
//! 2. `/proc/modules` + `/sys/module/<name>/` — loaded-module detection.
//! 3. `/lib/modules/$(uname -r)/modules.builtin` (preferred) or
//!    `modules.builtin.bin` — built-in detection.
//! 4. `/boot/config-$(uname -r)` or `/proc/config.gz` — secondary
//!    `CONFIG_*` evidence only.
//!
//! `br_netfilter` post-step-2 detection drives the
//! `bridge-nf-call-iptables=0` / `bridge-nf-call-ip6tables=0`
//! recommendation surfaced in the probe result.
//!
//! Mutation (`modprobe`) lives in the broker — see
//! `nixling_priv_broker::ops::modprobe`. This module is pure read-only
//! preflight and exposes deterministic parsers so the canary matrix can
//! drive it with fixtures.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nixling_core::host_w3::{KernelModuleEntry, ModuleRequirementW3};
use serde::{Deserialize, Serialize};

/// Set of modules currently loaded into the running kernel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedModuleSet {
    pub names: BTreeSet<String>,
}

impl LoadedModuleSet {
    /// Parses the `/proc/modules` line format: one record per line,
    /// fields whitespace-separated; the first field is the module
    /// name.
    pub fn parse_proc_modules(contents: &str) -> Self {
        let mut names = BTreeSet::new();
        for line in contents.lines() {
            if let Some(name) = line.split_whitespace().next() {
                if !name.is_empty() {
                    names.insert(name.to_owned());
                }
            }
        }
        Self { names }
    }

    pub fn contains(&self, module: &str) -> bool {
        self.names.contains(module)
    }
}

/// Set of modules built directly into the running kernel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinModuleSet {
    pub names: BTreeSet<String>,
}

impl BuiltinModuleSet {
    /// Parses `modules.builtin`: one relative path per line; the
    /// module name is the basename minus the `.ko` (or `.ko.xz`,
    /// `.ko.zst`) suffix.
    pub fn parse_modules_builtin(contents: &str) -> Self {
        let mut names = BTreeSet::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let base = Path::new(line)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(line);
            let stem = base
                .trim_end_matches(".xz")
                .trim_end_matches(".zst")
                .trim_end_matches(".gz")
                .trim_end_matches(".ko");
            if !stem.is_empty() {
                names.insert(stem.to_owned());
            }
        }
        Self { names }
    }

    /// Parses the in-kernel `modules.builtin.bin` format (the binary
    /// sibling of `modules.builtin`). The file is a concatenation of
    /// null-terminated records `<key>=<value>\0`; per-module records
    /// share a `<module-relpath>.<info>=<value>` shape with the module
    /// path acting as the prefix before the first `.` in the key.
    /// Older kernels (depmod ≤ 5.x without `--symbol-prefix`) store
    /// just `<module-relpath>\0` records; we accept both.
    ///
    /// We extract the unique set of module relpaths (anything ending
    /// in `.ko`, `.ko.xz`, `.ko.zst`, or `.ko.gz`) and reuse the
    /// basename stemming pass from [`parse_modules_builtin`].
    pub fn parse_modules_builtin_bin(bytes: &[u8]) -> Self {
        let mut names = BTreeSet::new();
        for record in bytes.split(|b| *b == 0) {
            if record.is_empty() {
                continue;
            }
            let Ok(text) = std::str::from_utf8(record) else {
                continue;
            };
            // Record shape is either `<relpath>` or `<relpath>.<info>=<value>`.
            // Cut at the first `=` to drop the value, then look for a
            // suffix that names a module.
            let lhs = text.split_once('=').map(|(l, _)| l).unwrap_or(text);
            let key = lhs.rsplit('/').next().unwrap_or(lhs);
            let candidate = key.split('.').next().unwrap_or(key);
            // Also handle the legacy `<relpath>` (no `.info`) form by
            // running the modules.builtin-style stemming pass on the
            // whole record.
            let stems = [
                candidate,
                lhs.rsplit('/').next().unwrap_or(lhs),
                Path::new(lhs)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(lhs),
            ];
            for stem in stems {
                let s = stem
                    .trim_end_matches(".xz")
                    .trim_end_matches(".zst")
                    .trim_end_matches(".gz")
                    .trim_end_matches(".ko");
                if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    names.insert(s.to_owned());
                    break;
                }
            }
        }
        Self { names }
    }

    pub fn contains(&self, module: &str) -> bool {
        self.names.contains(module)
    }
}

/// Parsed `CONFIG_*=<value>` snapshot from `/boot/config-$(uname -r)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelConfig {
    pub entries: std::collections::BTreeMap<String, String>,
}

impl KernelConfig {
    /// Parses the `CONFIG_KEY=value` text format. Lines starting with
    /// `#` (including `# CONFIG_X is not set`) are skipped.
    pub fn parse(contents: &str) -> Self {
        let mut entries = std::collections::BTreeMap::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                entries.insert(k.to_owned(), v.to_owned());
            }
        }
        Self { entries }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }
}

/// Reads `/proc/sys/kernel/modules_disabled`; returns `true` only when
/// the file exists and its trimmed value is `1`. Any read failure is
/// treated as "modules are not locked" since the probe is paired with
/// the loaded/builtin checks that fail closed independently.
pub fn probe_modules_disabled() -> bool {
    probe_modules_disabled_at(Path::new("/proc/sys/kernel/modules_disabled"))
}

/// Test-shim variant of [`probe_modules_disabled`] reading a caller-
/// supplied path.
pub fn probe_modules_disabled_at(path: &Path) -> bool {
    matches!(fs::read_to_string(path), Ok(s) if s.trim() == "1")
}

/// Reads `/proc/modules` plus `/sys/module/*` for loaded-module
/// detection.
pub fn read_loaded_modules() -> LoadedModuleSet {
    read_loaded_modules_at(Path::new("/proc/modules"), Path::new("/sys/module"))
}

/// Test-shim variant of [`read_loaded_modules`].
pub fn read_loaded_modules_at(proc_modules: &Path, sys_module_dir: &Path) -> LoadedModuleSet {
    let mut set = match fs::read_to_string(proc_modules) {
        Ok(contents) => LoadedModuleSet::parse_proc_modules(&contents),
        Err(_) => LoadedModuleSet::default(),
    };
    if let Ok(entries) = fs::read_dir(sys_module_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                set.names.insert(name.to_owned());
            }
        }
    }
    set
}

/// Reads `/lib/modules/$(uname -r)/modules.builtin`. Returns an empty
/// set on failure; the production probe order falls back to
/// `modules.builtin.bin` via [`read_builtin_modules_with_fallback`]
/// in step 3.
pub fn read_builtin_modules() -> BuiltinModuleSet {
    let release = uname_release().unwrap_or_default();
    let primary = PathBuf::from(format!("/lib/modules/{release}/modules.builtin"));
    read_builtin_modules_at(&primary)
}

/// Two-stage builtin probe: prefers `modules.builtin` (text), falls
/// back to `modules.builtin.bin` (in-kernel format) when the text
/// variant is missing or unparseable. Returns the union of both if
/// both parse successfully.
pub fn read_builtin_modules_with_fallback() -> BuiltinModuleSet {
    let release = uname_release().unwrap_or_default();
    let primary = PathBuf::from(format!("/lib/modules/{release}/modules.builtin"));
    let fallback = PathBuf::from(format!("/lib/modules/{release}/modules.builtin.bin"));
    read_builtin_modules_with_fallback_at(&primary, &fallback)
}

/// Test-shim variant of [`read_builtin_modules`].
pub fn read_builtin_modules_at(path: &Path) -> BuiltinModuleSet {
    match fs::read_to_string(path) {
        Ok(contents) => BuiltinModuleSet::parse_modules_builtin(&contents),
        Err(_) => BuiltinModuleSet::default(),
    }
}

/// Test-shim variant of [`read_builtin_modules_with_fallback`].
pub fn read_builtin_modules_with_fallback_at(primary: &Path, fallback: &Path) -> BuiltinModuleSet {
    let primary_set = match fs::read_to_string(primary) {
        Ok(contents) => BuiltinModuleSet::parse_modules_builtin(&contents),
        Err(_) => BuiltinModuleSet::default(),
    };
    let fallback_set = match fs::read(fallback) {
        Ok(bytes) => BuiltinModuleSet::parse_modules_builtin_bin(&bytes),
        Err(_) => BuiltinModuleSet::default(),
    };
    let mut merged = primary_set.names;
    merged.extend(fallback_set.names);
    BuiltinModuleSet { names: merged }
}

/// Reads the host kernel config. Tries `/boot/config-$(uname -r)`
/// first; falls back to `/proc/config.gz` in step 4. Kernel config is treated
/// as **secondary** evidence; failure returns `None` and the
/// loaded+builtin path drives the decision.
pub fn read_kernel_config() -> Option<KernelConfig> {
    let release = uname_release()?;
    let primary = PathBuf::from(format!("/boot/config-{release}"));
    let fallback = PathBuf::from("/proc/config.gz");
    read_kernel_config_with_fallback_at(&primary, &fallback)
}

/// Test-shim variant of [`read_kernel_config`].
pub fn read_kernel_config_at(path: &Path) -> Option<KernelConfig> {
    fs::read_to_string(path)
        .ok()
        .map(|s| KernelConfig::parse(&s))
}

/// Two-stage kernel-config probe: prefers `<primary>` (uncompressed),
/// falls back to a `<fallback>` gzip-encoded blob (`/proc/config.gz`).
/// Returns `None` only if neither source yields a parseable config.
pub fn read_kernel_config_with_fallback_at(
    primary: &Path,
    fallback: &Path,
) -> Option<KernelConfig> {
    if let Ok(contents) = fs::read_to_string(primary) {
        return Some(KernelConfig::parse(&contents));
    }
    let bytes = fs::read(fallback).ok()?;
    let decoded = gunzip_inflate(&bytes).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    Some(KernelConfig::parse(&text))
}

/// Minimal RFC 1952 gzip → DEFLATE → text decoder used by the
/// `/proc/config.gz` fallback. Strips the gzip header (magic +
/// optional FEXTRA / FNAME / FCOMMENT) and the trailing 8-byte
/// CRC32 + ISIZE, then hands the raw DEFLATE stream to
/// [`miniz_oxide`]. Pure: callers feed a `&[u8]` so the test path
/// drives both the success and the malformed-header branches.
pub fn gunzip_inflate(bytes: &[u8]) -> Result<Vec<u8>, GzipError> {
    // RFC 1952 §2.3.1: minimum gzip stream is 10-byte fixed header
    // + 8-byte trailer.
    if bytes.len() < 18 {
        return Err(GzipError::TooShort);
    }
    if bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err(GzipError::MissingMagic);
    }
    if bytes[2] != 8 {
        // CM = 8 (DEFLATE) is the only compression method specified.
        return Err(GzipError::UnsupportedMethod(bytes[2]));
    }
    let flags = bytes[3];
    let mut offset = 10usize;
    // FEXTRA (0x04): SI1+SI2+XLEN (2 bytes LE) + XLEN bytes of data.
    if flags & 0x04 != 0 {
        if offset + 2 > bytes.len() {
            return Err(GzipError::Truncated);
        }
        let xlen = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        offset = offset.checked_add(2 + xlen).ok_or(GzipError::Truncated)?;
    }
    // FNAME (0x08): null-terminated string.
    if flags & 0x08 != 0 {
        offset = skip_cstring(bytes, offset)?;
    }
    // FCOMMENT (0x10): null-terminated string.
    if flags & 0x10 != 0 {
        offset = skip_cstring(bytes, offset)?;
    }
    // FHCRC (0x02): 2 bytes header CRC16.
    if flags & 0x02 != 0 {
        offset = offset.checked_add(2).ok_or(GzipError::Truncated)?;
    }
    if offset + 8 > bytes.len() {
        return Err(GzipError::Truncated);
    }
    let payload = &bytes[offset..bytes.len() - 8];
    miniz_oxide::inflate::decompress_to_vec(payload).map_err(|err| GzipError::Inflate {
        detail: format!("{err:?}"),
    })
}

fn skip_cstring(bytes: &[u8], start: usize) -> Result<usize, GzipError> {
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if *b == 0 {
            return Ok(i + 1);
        }
    }
    Err(GzipError::Truncated)
}

/// Errors emitted by [`gunzip_inflate`]. Stays in this module because
/// kernel-config is secondary evidence — callers map every variant to
/// `None` and fall through to the loaded+builtin path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GzipError {
    TooShort,
    MissingMagic,
    UnsupportedMethod(u8),
    Truncated,
    Inflate { detail: String },
}

fn uname_release() -> Option<String> {
    let raw = rustix::system::uname();
    raw.release().to_str().ok().map(|s| s.to_owned())
}

/// Disposition for a single module entry after the four-step probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModuleDisposition {
    /// Module is already loaded.
    Loaded,
    /// Module is compiled into the running kernel.
    Builtin,
    /// Module is absent but optional (no fail-closed).
    OptionalAbsent,
    /// Module is absent and `modules_disabled=1` blocks loading.
    HostModulesLocked,
    /// Module is absent; broker can attempt `ModprobeIfAllowed`.
    Loadable,
}

/// Per-entry outcome of [`probe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleProbeRow {
    pub module: String,
    pub matrix_entry_id: String,
    pub requirement: ModuleRequirementW3,
    pub disposition: ModuleDisposition,
}

/// Aggregate four-step probe result. The result carries enough context
/// for the broker dispatcher to refuse a `ModprobeIfAllowed` request
/// without re-parsing /proc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleProbeResult {
    pub modules_disabled: bool,
    pub rows: Vec<ModuleProbeRow>,
    /// Names that failed the 4-step probe in a fail-closed way when
    /// `modules_disabled=1` prevented loading.
    pub host_modules_locked: Vec<String>,
    /// Whether `br_netfilter` was detected loaded or built-in. Drives
    /// the bridge-nf-call sysctl recommendation downstream.
    pub br_netfilter_present: bool,
    /// Per-link sysctl recommendations the probe surfaces when
    /// `br_netfilter` is present.
    pub bridge_nf_recommendations: Vec<BridgeNfRecommendation>,
}

/// Recommended sysctl override surfaced when `br_netfilter` is in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeNfRecommendation {
    pub key: String,
    pub recommended: String,
    pub rationale: String,
}

/// Inputs to [`probe_with`] — bundled so the L1c canaries can drive
/// the deterministic probe without touching `/proc`.
#[derive(Debug, Clone)]
pub struct ProbeInputs {
    pub modules_disabled: bool,
    pub loaded: LoadedModuleSet,
    pub builtin: BuiltinModuleSet,
}

/// Real-host wrapper around [`probe_with`]. Reads `/proc` + `/sys`
/// plus the modules.builtin two-stage probe (text first, then
/// `modules.builtin.bin`).
pub fn probe(kmodules: &[KernelModuleEntry]) -> ModuleProbeResult {
    probe_with(
        kmodules,
        &ProbeInputs {
            modules_disabled: probe_modules_disabled(),
            loaded: read_loaded_modules(),
            builtin: read_builtin_modules_with_fallback(),
        },
    )
}

/// Pure deterministic four-step probe used by the broker and by the
/// L1c canary tests.
pub fn probe_with(kmodules: &[KernelModuleEntry], inputs: &ProbeInputs) -> ModuleProbeResult {
    let mut rows = Vec::with_capacity(kmodules.len());
    let mut locked = Vec::new();
    for entry in kmodules {
        let disposition = if inputs.loaded.contains(&entry.module) {
            ModuleDisposition::Loaded
        } else if inputs.builtin.contains(&entry.module) {
            ModuleDisposition::Builtin
        } else {
            let fail_when_locked = match entry.requirement {
                ModuleRequirementW3::Required => true,
                ModuleRequirementW3::Alternatives
                | ModuleRequirementW3::Optional
                | ModuleRequirementW3::Deferred => entry.fail_if_modules_disabled,
            };
            if inputs.modules_disabled {
                if fail_when_locked {
                    locked.push(entry.module.clone());
                    ModuleDisposition::HostModulesLocked
                } else {
                    ModuleDisposition::OptionalAbsent
                }
            } else if matches!(
                entry.requirement,
                ModuleRequirementW3::Required | ModuleRequirementW3::Alternatives
            ) {
                ModuleDisposition::Loadable
            } else {
                ModuleDisposition::OptionalAbsent
            }
        };
        rows.push(ModuleProbeRow {
            module: entry.module.clone(),
            matrix_entry_id: entry.matrix_entry_id.clone(),
            requirement: entry.requirement.clone(),
            disposition,
        });
    }

    let br_netfilter_present =
        inputs.loaded.contains("br_netfilter") || inputs.builtin.contains("br_netfilter");
    let bridge_nf_recommendations = if br_netfilter_present {
        vec![
            BridgeNfRecommendation {
                key: "net.bridge.bridge-nf-call-iptables".to_owned(),
                recommended: "0".to_owned(),
                rationale: "br_netfilter loaded; pin to 0 so iptables cannot route around inet nixling unless ADR opts in".to_owned(),
            },
            BridgeNfRecommendation {
                key: "net.bridge.bridge-nf-call-ip6tables".to_owned(),
                recommended: "0".to_owned(),
                rationale: "br_netfilter loaded; pin to 0 so ip6tables cannot route around inet nixling unless ADR opts in".to_owned(),
            },
        ]
    } else {
        Vec::new()
    };

    ModuleProbeResult {
        modules_disabled: inputs.modules_disabled,
        rows,
        host_modules_locked: locked,
        br_netfilter_present,
        bridge_nf_recommendations,
    }
}

/// Convenience: returns `true` if any row in the probe locked closed.
impl ModuleProbeResult {
    pub fn fail_closed(&self) -> bool {
        !self.host_modules_locked.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(module: &str, req: ModuleRequirementW3, fail_locked: bool) -> KernelModuleEntry {
        KernelModuleEntry {
            module: module.to_owned(),
            matrix_entry_id: format!("matrix-{module}"),
            feature: "test".to_owned(),
            requirement: req,
            fail_if_modules_disabled: fail_locked,
        }
    }

    #[test]
    fn parse_proc_modules_extracts_names() {
        let txt = "kvm_intel 1234 0 - Live 0x0\nbr_netfilter 100 0 - Live 0x0\n";
        let set = LoadedModuleSet::parse_proc_modules(txt);
        assert!(set.contains("kvm_intel"));
        assert!(set.contains("br_netfilter"));
    }

    #[test]
    fn parse_modules_builtin_strips_path_and_compression_suffix() {
        let txt = "kernel/drivers/net/tun.ko\nkernel/foo/kvm.ko.xz\n#comment\nkernel/bar/vhost_net.ko.zst\n";
        let set = BuiltinModuleSet::parse_modules_builtin(txt);
        assert!(set.contains("tun"));
        assert!(set.contains("kvm"));
        assert!(set.contains("vhost_net"));
    }

    #[test]
    fn kernel_config_skips_not_set_comments() {
        let txt = "# CONFIG_X is not set\nCONFIG_KVM=y\nCONFIG_TUN=m\n";
        let kc = KernelConfig::parse(txt);
        assert_eq!(kc.get("CONFIG_KVM"), Some("y"));
        assert_eq!(kc.get("CONFIG_TUN"), Some("m"));
        assert!(kc.get("CONFIG_X").is_none());
    }

    #[test]
    fn modules_disabled_locks_required_absent_module() {
        let inputs = ProbeInputs {
            modules_disabled: true,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet::default(),
        };
        let result = probe_with(
            &[entry("kvm", ModuleRequirementW3::Required, true)],
            &inputs,
        );
        assert!(result.fail_closed());
        assert_eq!(result.host_modules_locked, vec!["kvm".to_owned()]);
        assert_eq!(
            result.rows[0].disposition,
            ModuleDisposition::HostModulesLocked
        );
    }

    #[test]
    fn required_module_locks_even_when_fail_flag_is_false() {
        let inputs = ProbeInputs {
            modules_disabled: true,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet::default(),
        };
        let result = probe_with(
            &[entry("kvm", ModuleRequirementW3::Required, false)],
            &inputs,
        );
        assert!(result.fail_closed());
        assert_eq!(result.host_modules_locked, vec!["kvm".to_owned()]);
        assert_eq!(
            result.rows[0].disposition,
            ModuleDisposition::HostModulesLocked
        );
    }

    #[test]
    fn loaded_module_passes_even_with_modules_disabled() {
        let inputs = ProbeInputs {
            modules_disabled: true,
            loaded: LoadedModuleSet {
                names: ["kvm".to_owned()].into_iter().collect(),
            },
            builtin: BuiltinModuleSet::default(),
        };
        let result = probe_with(
            &[entry("kvm", ModuleRequirementW3::Required, true)],
            &inputs,
        );
        assert!(!result.fail_closed());
        assert_eq!(result.rows[0].disposition, ModuleDisposition::Loaded);
    }

    #[test]
    fn builtin_module_passes_even_with_modules_disabled() {
        let inputs = ProbeInputs {
            modules_disabled: true,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet {
                names: ["kvm".to_owned()].into_iter().collect(),
            },
        };
        let result = probe_with(
            &[entry("kvm", ModuleRequirementW3::Required, true)],
            &inputs,
        );
        assert!(!result.fail_closed());
        assert_eq!(result.rows[0].disposition, ModuleDisposition::Builtin);
    }

    #[test]
    fn optional_module_respects_fail_if_modules_disabled_flag() {
        let inputs = ProbeInputs {
            modules_disabled: true,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet::default(),
        };

        let relaxed = probe_with(
            &[entry("vfio", ModuleRequirementW3::Optional, false)],
            &inputs,
        );
        assert!(!relaxed.fail_closed());
        assert!(relaxed.host_modules_locked.is_empty());
        assert_eq!(
            relaxed.rows[0].disposition,
            ModuleDisposition::OptionalAbsent
        );

        let strict = probe_with(
            &[entry("vfio", ModuleRequirementW3::Optional, true)],
            &inputs,
        );
        assert!(strict.fail_closed());
        assert_eq!(strict.host_modules_locked, vec!["vfio".to_owned()]);
        assert_eq!(
            strict.rows[0].disposition,
            ModuleDisposition::HostModulesLocked
        );
    }

    #[test]
    fn br_netfilter_loaded_triggers_bridge_nf_recommendations() {
        let inputs = ProbeInputs {
            modules_disabled: false,
            loaded: LoadedModuleSet {
                names: ["br_netfilter".to_owned()].into_iter().collect(),
            },
            builtin: BuiltinModuleSet::default(),
        };
        let result = probe_with(&[], &inputs);
        assert!(result.br_netfilter_present);
        assert_eq!(result.bridge_nf_recommendations.len(), 2);
        assert!(result
            .bridge_nf_recommendations
            .iter()
            .any(|r| r.key == "net.bridge.bridge-nf-call-iptables"));
    }

    #[test]
    fn loadable_when_modules_not_disabled_and_absent() {
        let inputs = ProbeInputs {
            modules_disabled: false,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet::default(),
        };
        let result = probe_with(
            &[entry("tun", ModuleRequirementW3::Required, true)],
            &inputs,
        );
        assert_eq!(result.rows[0].disposition, ModuleDisposition::Loadable);
        assert!(!result.fail_closed());
    }

    #[test]
    fn parse_modules_builtin_bin_extracts_legacy_relpath_records() {
        // Legacy depmod (no `--symbol-prefix`): records are bare
        // relpaths separated by NULs.
        let blob = b"kernel/drivers/net/tun.ko\0kernel/fs/fuse/fuse.ko.xz\0\0";
        let set = BuiltinModuleSet::parse_modules_builtin_bin(blob);
        assert!(set.contains("tun"));
        assert!(set.contains("fuse"));
    }

    #[test]
    fn parse_modules_builtin_bin_extracts_keyed_records() {
        // Modern depmod: records carry `<relpath>.<info>=<value>`.
        let blob = b"kernel/drivers/net/tun.ko.alias=tun-x\0kernel/fs/fuse/fuse.ko.alias=fuse\0";
        let set = BuiltinModuleSet::parse_modules_builtin_bin(blob);
        assert!(set.contains("tun"));
        assert!(set.contains("fuse"));
    }

    #[test]
    fn read_builtin_modules_with_fallback_uses_bin_when_text_missing() {
        // Drive the test-shim variant with a non-existent primary so
        // the fallback path is exercised end-to-end.
        let dir = std::env::temp_dir().join(format!(
            "nixling-w3fu1-h2-modules-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let primary = dir.join("modules.builtin"); // intentionally absent
        let fallback = dir.join("modules.builtin.bin");
        std::fs::write(&fallback, b"kernel/drivers/net/tun.ko\0").unwrap();
        let set = read_builtin_modules_with_fallback_at(&primary, &fallback);
        assert!(set.contains("tun"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gunzip_inflate_round_trip_against_known_deflate_stream() {
        // Construct a tiny gzip stream by prepending the RFC1952 fixed
        // header to a precomputed raw DEFLATE block produced by the
        // standard zlib compressor for the text `CONFIG_KVM=y\n`. The
        // raw block here is the stored (BTYPE=00) form — a valid
        // DEFLATE block with no compression that miniz_oxide will
        // emit verbatim.
        let payload = b"CONFIG_KVM=y\n";
        let mut stored_block = Vec::new();
        // BFINAL=1, BTYPE=00 (stored); LEN/NLEN; then literal bytes.
        stored_block.push(0x01);
        let len = payload.len() as u16;
        stored_block.extend_from_slice(&len.to_le_bytes());
        stored_block.extend_from_slice(&(!len).to_le_bytes());
        stored_block.extend_from_slice(payload);

        let mut gz = vec![
            0x1f, 0x8b, // magic
            0x08, // CM = deflate
            0x00, // no flags
            0, 0, 0, 0,    // mtime
            0x00, // XFL
            0xff, // OS
        ];
        gz.extend_from_slice(&stored_block);
        // 8-byte trailer (CRC32 + ISIZE) — values are not validated by
        // `gunzip_inflate`, so any 8 bytes will do.
        gz.extend_from_slice(&[0u8; 8]);

        let out = gunzip_inflate(&gz).expect("gunzip ok");
        assert_eq!(out, payload);
    }

    #[test]
    fn gunzip_inflate_refuses_missing_magic() {
        let bad = [0u8; 32];
        let err = gunzip_inflate(&bad).unwrap_err();
        assert_eq!(err, GzipError::MissingMagic);
    }

    #[test]
    fn read_kernel_config_with_fallback_prefers_primary() {
        let dir = std::env::temp_dir().join(format!(
            "nixling-w3fu1-h2-kconfig-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let primary = dir.join("config-x");
        let fallback = dir.join("config.gz");
        std::fs::write(&primary, "CONFIG_KVM=y\n").unwrap();
        // Fallback would refuse to inflate (empty), proving primary wins.
        std::fs::write(&fallback, b"").unwrap();
        let kc = read_kernel_config_with_fallback_at(&primary, &fallback).unwrap();
        assert_eq!(kc.get("CONFIG_KVM"), Some("y"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
