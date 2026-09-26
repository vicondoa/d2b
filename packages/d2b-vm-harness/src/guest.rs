//! Booting one check's guest, and taking it down again.
//!
//! The launcher is the half of the lane that replaces the nix test driver's
//! boot responsibility. It reproduces the emulator invocation the check's
//! guest configuration declared - memory, vCPU count, the drive layout
//! including the writable-store root drive, and whatever per-check device
//! options the node contributed - boots it, waits for the guest's own
//! activation contract, and tears it down.
//!
//! Three properties are worth naming, because each is a decision rather than
//! a detail:
//!
//! * The accelerator is selected explicitly. `-accel kvm` with no TCG
//!   fallback is the difference between a host that runs the lane and a host
//!   that runs it six times slower without saying so.
//! * The root drive is copied into a working directory the lane owns. The
//!   image is a read-only graph output, the guest writes to its disk, and
//!   `snapshot=on` state disks put their per-guest overlays under the
//!   emulator's `TMPDIR` - which therefore has to be that same directory and
//!   has to survive the run.
//! * Every reusable-pool guest carries a machine identity device and a
//!   hardware random source. Without them a restored guest replays one RNG
//!   stream, and the guest's own activation marker is only written once its
//!   random pool is initialised, so a snapshot can never be taken cold.

use std::{
    ffi::OsString,
    fs,
    io::Write,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use crate::{
    error::{HarnessError, Result, UnsnapshottableDevice},
    manifest::GuestManifest,
    monitor::Monitor,
};

/// The longest a guest's own directory may be while still carrying a Unix
/// socket. The limit is 108 bytes; the headroom covers the socket name the
/// emulator appends.
const WORK_DIR_PATH_BUDGET: usize = 88;

/// The short, lane-scoped root a guest falls back to when its own directory
/// cannot carry a socket.
const LANE_TEMP_DIR: &str = "d2b-vm-lane";

/// The guest's command channel: the chardev the launch declares it under,
/// and the socket file that chardev connects to, in the working directory
/// the guest owns.
///
/// These are the launcher's, not a caller's: the console is part of the
/// invocation, so the socket path is a function of the guest's own working
/// directory and the chardev id is a name the command line and the guest's
/// surface have to agree on. The guest side of the channel is the nix test
/// framework's `backdoor.service` - a root shell on this console - which
/// the guest image carries.
pub const CONSOLE_ID: &str = "d2b-lane-console";
pub const CONSOLE_SOCKET: &str = "lane-console.sock";

/// How often the launcher re-reads the guest's console while waiting for
/// activation.
const CONSOLE_POLL: Duration = Duration::from_millis(500);

/// How much of the guest's console travels with an activation failure. A
/// bounded wait that reports nothing is the hang the bound replaced.
const CONSOLE_TAIL_BYTES: usize = 64 * 1024;
/// The lines the guest announces its ordering report between. A guest that
/// never activates has not necessarily failed: a unit ordered after one that
/// never started never enters `failed` at all, so the boot's failed set can
/// be empty while the unit the contract is waiting for sits in `activating`
/// behind an ordering dependency nobody has named. The guest therefore
/// reports the graph it is sitting in, and these delimiters are what let the
/// launcher lift that report out of the console rather than leaving the next
/// reader to reproduce a twenty-minute run to see it.
const ORDERING_REPORT_OPENS: &str = "d2b-lane-activation: ordering state for ";
const ORDERING_REPORT_CLOSES: &str = "d2b-lane-activation: end ordering state";

/// Everything one guest needs in order to be booted exactly as its
/// configuration declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSpec {
    /// The invocation the guest image declared.
    pub manifest: GuestManifest,
    /// The built image the manifest came from.
    pub image_dir: PathBuf,
    /// The emulator binary, taken from the pinned nix package set the guest
    /// closure was realized from.
    pub emulator: PathBuf,
    /// The lane's working directory. Each guest gets a subdirectory of it
    /// that it owns for its whole life and removes on teardown.
    pub work_root: PathBuf,
    /// How long the guest has to report activation before the lane fails.
    pub activation_timeout: Duration,
    /// The guest's name on the monitor and in diagnostics.
    pub name: String,
}

impl GuestSpec {
    /// Build a spec for an image, with the lane's own defaults.
    pub fn new(
        manifest: GuestManifest,
        image_dir: impl Into<PathBuf>,
        emulator: impl Into<PathBuf>,
        work_root: impl Into<PathBuf>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            manifest,
            image_dir: image_dir.into(),
            emulator: emulator.into(),
            work_root: work_root.into(),
            activation_timeout: Duration::from_secs(1200),
            name: name.into(),
        }
    }

    /// The directory this guest owns.
    ///
    /// The monitor socket the launcher talks to the emulator over lives in
    /// this directory, and a Unix socket path is limited to 108 bytes. Under
    /// Bazel the runfiles path is far longer than that, so a guest whose
    /// directory would be too long to carry a socket is given a short
    /// lane-scoped directory under the host's temporary directory instead.
    /// It is created, owned, and removed exactly like the other one; only its
    /// location differs, and `work_root` is a caller choice precisely so it
    /// can be.
    pub fn work_dir(&self) -> PathBuf {
        let directory = self.work_root.join(&self.name);
        if directory.as_os_str().len() < WORK_DIR_PATH_BUDGET {
            return directory;
        }
        std::env::temp_dir()
            .join(LANE_TEMP_DIR)
            .join(&self.name)
    }

    /// The emulator's command line, exactly as the guest's configuration
    /// declared it.
    ///
    /// This is a pure function of the manifest and the guest's own working
    /// directory, which is what makes the per-check invocation assertable
    /// without booting anything: a node that asked for a different memory
    /// size, a different vCPU count, a different disk, or an extra device
    /// produces a different command line, and that difference is the check.
    pub fn command_line(&self) -> Result<Vec<OsString>> {
        let manifest = &self.manifest;
        let work_dir = self.work_dir();
        let mut argv: Vec<OsString> = Vec::new();
        let mut push = |argument: &str| argv.push(OsString::from(argument));

        push(&self.emulator.to_string_lossy());
        push("-name");
        push(&self.name);

        // Hardware virtualization, explicitly, with no emulation fallback.
        // `max` is the CPU model the VM module launches with, and it is what
        // exposes virtualization extensions to the guest, which is how a
        // guest can run the nested guest the Cloud Hypervisor checks boot.
        push("-machine");
        push("accel=kvm");
        push("-cpu");
        push("max");

        push("-m");
        push(&manifest.machine.memory_size_mib.to_string());
        push("-smp");
        push(&manifest.machine.cores.to_string());

        // A machine identity device and a hardware random source, for every
        // guest. The identity device changes on every restore and forces the
        // guest to reseed; the random device is what the guest's activation
        // marker waits for before the lane is allowed to snapshot it.
        push("-device");
        push("vmgenid,guid=auto");
        push("-device");
        push("virtio-rng-pci");

        // The networking the node declared, in the position the VM module's
        // own run script gives it: after the random device, before the
        // drives. These are not cosmetic. A guest booted without them has no
        // network device at all, so the kernel never loads the module that
        // device needs and the daemon refuses to start over exactly that - a
        // guest that boots cleanly and then stops itself.
        for option in render_device_options(&manifest.networking_options) {
            push(&option.to_string_lossy());
        }

        // The emulator's runtime data - firmware blobs, keymaps, device
        // ROMs - lives beside the binary in the nix package set it came
        // from. The runfile is a symlink into that set, so the data
        // directory is named explicitly rather than left to the emulator's
        // own search, which a relocated runfiles tree would defeat.
        if let Some(data_dir) = self.emulator_data_dir() {
            push("-L");
            push(&data_dir);
        }

        // The lane's own plumbing: no display, the guest's console captured
        // for the activation wait and for diagnostics, a monitor socket the
        // launcher owns, and the guest's command channel.
        //
        // The channel is on the command line for the same reason the nix test
        // driver put it there. Its guest side is a unit that declares
        // `requires = [ "dev-hvc0.device" ]`, and a unit whose device is
        // absent when systemd reaches it is not reliably restarted when the
        // device turns up later: a console hot-attached through the monitor
        // after the boot leaves a guest whose root shell never ran. So the
        // device exists before the guest executes its first instruction, and
        // the host is already listening before the emulator starts at all -
        // a chardev with nobody on the other end of its socket blocks the
        // launch rather than reaching the guest.
        push("-display");
        push("none");
        push("-serial");
        push(&format!("file:{}", work_dir.join("console.log").display()));
        push("-monitor");
        push("none");
        push("-qmp");
        push(&format!(
            "unix:{},server=on,wait=off",
            work_dir.join("qmp.sock").display()
        ));

        push("-chardev");
        push(&format!(
            "socket,id={CONSOLE_ID},path={}",
            work_dir.join(CONSOLE_SOCKET).display()
        ));
        push("-device");
        push("virtio-serial");
        push("-device");
        push(&format!("virtconsole,chardev={CONSOLE_ID}"));

        // The guest's clock follows the emulator's virtual clock, which
        // stops while the guest does. That is what keeps a restored guest's
        // view of elapsed time continuous across a restore.
        push("-rtc");
        push("base=utc,clock=vm");

        for (index, drive) in manifest.drives.iter().enumerate() {
            let file = self.drive_file(drive.file.as_str(), &work_dir);
            let drive_id = format!("lane_drive_{index}");
            let mut drive_options = vec![
                ("index".to_owned(), index.to_string()),
                ("id".to_owned(), drive_id.clone()),
                ("if".to_owned(), "none".to_owned()),
                ("file".to_owned(), file),
            ];
            if let Some(format) = &drive.format {
                drive_options.push(("format".to_owned(), format.clone()));
            }
            drive_options.push(("cache".to_owned(), drive.cache.clone()));
            drive_options.push(("werror".to_owned(), drive.werror.clone()));
            push("-drive");
            push(&render_options(&drive_options));

            let mut device_options = vec![("drive".to_owned(), drive_id)];
            if let Some(boot_index) = &drive.boot_index {
                device_options.push(("bootindex".to_owned(), boot_index.clone()));
            }
            if let Some(serial) = &drive.serial {
                device_options.push(("serial".to_owned(), serial.clone()));
            }
            push("-device");
            push(&format!(
                "{},{}",
                device_model(&drive.interface),
                render_options(&device_options)
            ));
        }

        for share in &manifest.shared_directories {
            push("-virtfs");
            push(&format!(
                "local,path={},security_model={},mount_tag={}",
                resolve_share(&share.source, &work_dir)?,
                share.security_model,
                share.mount_tag
            ));
        }

        match manifest.boot.method.as_str() {
            "direct" => {
                let (Some(kernel), Some(initrd), Some(append)) = (
                    manifest.boot.kernel.as_deref(),
                    manifest.boot.initrd.as_deref(),
                    manifest.boot.append.as_deref(),
                ) else {
                    return Err(HarnessError::Configuration(
                        "the guest image declares the direct boot method without a kernel, an initrd, and a command line"
                            .to_owned(),
                    ));
                };
                push("-kernel");
                push(&manifest.resolve(&self.image_dir, kernel).to_string_lossy());
                push("-initrd");
                push(&manifest.resolve(&self.image_dir, initrd).to_string_lossy());
                push("-append");
                push(append);
            }
            "bootloader" => {
                if manifest.image.system_image.is_none() {
                    return Err(HarnessError::Configuration(
                        "the guest image declares the bootloader boot method without a system image to boot"
                            .to_owned(),
                    ));
                }
            }
            other => {
                return Err(HarnessError::Configuration(format!(
                    "the guest image declares the unknown boot method {other}"
                )));
            }
        }

        // The per-check device options come last, exactly where the node put
        // them: a vsock device, an extra drive, anything a check's own
        // configuration contributed rides through untouched.
        argv.extend(render_device_options(&manifest.extra_options));
        Ok(argv)
    }

    /// The emulator's runtime data directory, when the nix package set it
    /// came from carries one.
    ///
    /// The runfile is a symlink into the store, so the data directory is
    /// found beside the resolved binary rather than beside the runfile. A
    /// build of the emulator that ships no data directory - which is what a
    /// `qemu-kvm` wrapper built without the full data set looks like - gets
    /// no `-L` at all rather than a path that does not exist.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn emulator_data_dir(&self) -> Option<String> {
        let resolved = fs::canonicalize(&self.emulator).unwrap_or_else(|_| self.emulator.clone());
        let data_dir = resolved.parent()?.join("share").join("qemu");
        data_dir
            .is_dir()
            .then(|| data_dir.to_string_lossy().into_owned())
    }

    /// Where a declared drive file actually is at boot.
    ///
    /// The root drive is the guest's own: the image is a read-only graph
    /// output and the guest writes to it, so the launcher copies it into the
    /// working directory it owns. Everything else - a state disk, a shared
    /// image - is a store path the guest's node attached itself, and is
    /// used where the node put it.
    fn drive_file(&self, declared: &str, work_dir: &Path) -> String {
        if Path::new(declared).is_absolute() {
            declared.to_owned()
        } else {
            work_dir.join(declared).to_string_lossy().into_owned()
        }
    }

    /// Materialize the guest's working directory: the writable root drive,
    /// the scratch directory the guest's 9p shares are rooted at, and the
    /// emulator's `TMPDIR`.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn prepare_work_dir(&self) -> Result<PathBuf> {
        let work_dir = self.work_dir();
        if work_dir.exists() {
            fs::remove_dir_all(&work_dir)
                .map_err(|error| HarnessError::io(format!("clearing {}", work_dir.display()), error))?;
        }
        fs::create_dir_all(work_dir.join("xchg"))
            .map_err(|error| HarnessError::io(format!("creating {}", work_dir.display()), error))?;
        let disk = self.manifest.resolve(&self.image_dir, &self.manifest.image.disk);
        let guest_disk = work_dir.join(&self.manifest.image.disk);
        fs::copy(&disk, &guest_disk)
            .map_err(|error| HarnessError::io(format!("copying {} to {}", disk.display(), guest_disk.display()), error))?;
        // The image is a read-only graph output and the copy inherits that
        // mode, but the guest writes to its root drive: the emulator refuses
        // to open a drive it cannot write, and reports the permission rather
        // than the shape that caused it.
        fs::set_permissions(&guest_disk, std::fs::Permissions::from_mode(0o644))
            .map_err(|error| HarnessError::io(format!("making {} writable", guest_disk.display()), error))?;
        Ok(work_dir)
    }
}

/// Render the node's device-option list into arguments.
///
/// The list is shell text by construction, not a pre-split argument vector:
/// the NixOS VM module declares a whole flag and its value in one entry -
/// `"-device virtio-keyboard"` - and its own run script consumes the list
/// unquoted, so the shell word-splits it on the way to the emulator. The
/// launcher reproduces that split rather than handing the entries to
/// `execvp` as they stand: an entry that names a flag and its value is two
/// arguments to the emulator, and passing it as one makes the emulator reject
/// the entry with a message that names the device rather than the launcher.
/// An entry carrying its own properties, `-device usb-tablet,bus=usb-bus.0`,
/// has no whitespace and reaches the emulator whole.
fn render_device_options(options: &[String]) -> impl Iterator<Item = OsString> {
    options
        .iter()
        .flat_map(|option| option.split_whitespace())
        .map(OsString::from)
}

/// The block device model for a bus the node declared.
fn device_model(interface: &str) -> &'static str {
    match interface {
        "scsi" => "lsi53c895a",
        "ide" => "ide-hd",
        _ => "virtio-blk-pci",
    }
}

fn render_options(options: &[(String, String)]) -> String {
    options
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// A host directory the guest shares over 9p, with the VM module's
/// `TMPDIR`-relative sources resolved against the working directory the lane
/// owns.
///
/// The module declares two scratch shares that way - one rooted directly at
/// `TMPDIR`, one the caller can redirect through `SHARED_DIR` - and both name
/// the guest's own scratch directory. A source the lane cannot resolve to a
/// path is an error rather than a literal string handed to the emulator,
/// because the emulator reads an unresolved `9p` source as a directory that
/// does not exist and stops at machine start.
fn resolve_share(source: &str, work_dir: &Path) -> Result<String> {
    let declared = source.replace('"', "");
    let inner = declared
        .strip_prefix("${SHARED_DIR:-")
        .map(|rest| {
            rest.strip_suffix('}').ok_or_else(|| {
                HarnessError::Configuration(format!(
                    "the guest declares an unterminated share source: {source}"
                ))
            })
        })
        .transpose()?
        .unwrap_or(&declared);
    let relative = if let Some(rest) = inner.strip_prefix("$TMPDIR") {
        rest
    } else if inner.contains('$') {
        return Err(HarnessError::Configuration(format!(
            "the guest declares a share source the lane cannot resolve: {source}"
        )));
    } else {
        return Ok(inner.to_owned());
    };
    Ok(work_dir
        .join(relative.trim_start_matches('/'))
        .to_string_lossy()
        .into_owned())
}

/// Bind the host end of the guest's command channel.
///
/// The working directory is created by the launch that precedes this, so a
/// socket file still there is the working directory's own: it is removed
/// before the bind rather than around it, because a bind that fails on a
/// stale file would be a launch that failed for a reason its own diagnostics
/// do not name.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn bind_command_channel(work_dir: &Path) -> Result<UnixListener> {
    let path = work_dir.join(CONSOLE_SOCKET);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| HarnessError::io(format!("removing {}", path.display()), error))?;
    }
    UnixListener::bind(&path)
        .map_err(|error| HarnessError::io(format!("binding {}", path.display()), error))
}

/// A booted guest that has reported its activation contract.
pub struct ActiveGuest {
    child: Child,
    monitor: Option<Monitor>,
    work_dir: PathBuf,
    command_channel: Option<UnixListener>,
    shut_down: bool,
}

impl ActiveGuest {
    /// The guest's monitor, for the pool's snapshot and restore work.
    pub fn monitor(&mut self) -> Option<&mut Monitor> {
        self.monitor.as_mut()
    }

    /// Where the guest's console is being captured.
    pub fn console_log(&self) -> PathBuf {
        self.work_dir.join("console.log")
    }

    /// The working directory this guest owns.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Hand this guest's command channel to whoever will run a check's
    /// assertions against it.
    ///
    /// The socket was bound and put on the launch's command line before the
    /// emulator was started, and the emulator has connected to it since; what
    /// is left is accepting the connection and waiting for the guest's shell
    /// to announce itself. It is taken rather than borrowed because one guest
    /// carries one command channel, and a second reader of it would interleave
    /// two commands' output into one block.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn take_command_channel(&mut self) -> Result<UnixListener> {
        self.command_channel.take().ok_or_else(|| {
            HarnessError::Configuration(
                "the guest's command channel has already been handed to a guest-control surface"
                    .to_owned(),
            )
        })
    }

    /// Refuse a guest whose attached writable devices cannot carry an
    /// internal snapshot.
    ///
    /// This runs at boot, before any check does, because the alternative is
    /// discovering it at the first restore - after a suite has already run
    /// against a guest that could not be rolled back.
    pub fn require_snapshottable(&mut self) -> Result<()> {
        let devices = self
            .monitor
            .as_mut()
            .ok_or_else(|| HarnessError::Configuration("the guest has no monitor".to_owned()))?
            .block_devices()?;
        let unsnapshottable: Vec<UnsnapshottableDevice> = devices
            .iter()
            .filter(|device| !device.snapshottable)
            .map(|device| UnsnapshottableDevice {
                device: device.id.clone(),
                file: device.file.clone(),
                format: device.format.clone(),
            })
            .collect();
        if unsnapshottable.is_empty() {
            Ok(())
        } else {
            Err(HarnessError::NotSnapshottable {
                devices: unsnapshottable,
            })
        }
    }

    /// Take the pool's snapshot of this guest, under a tag of the lane's
    /// choosing.
    ///
    /// This is the point the pool snapshots at: after activation has
    /// completed, after every attached writable device has been proven to
    /// carry a snapshot, and before any check has touched the guest. A
    /// snapshot taken later would be a snapshot of whatever the last check
    /// left behind.
    pub fn save_snapshot(&mut self, tag: &str) -> Result<()> {
        self.monitor_mut()?.save_snapshot(tag)
    }

    /// Restore this guest from a snapshot it took itself.
    ///
    /// The guest's command channel survives the restore. An internal
    /// snapshot captures the guest's memory and its devices, not the host
    /// socket at the far end of the guest's console, and the emulator is the
    /// same process across the restore - so the connection the launcher
    /// accepted before the boot is still the connection the guest's root
    /// shell is reading from afterwards. That is also why the snapshot is
    /// taken with the channel idle: bytes already in the console are not
    /// part of what a restore rolls back.
    pub fn restore(&mut self, tag: &str) -> Result<()> {
        self.monitor_mut()?.load_snapshot(tag)
    }

    /// Whether this guest currently holds a snapshot under a tag.
    pub fn holds_snapshot(&mut self, tag: &str) -> Result<bool> {
        Ok(self.monitor_mut()?.snapshot_tags()?.iter().any(|held| held == tag))
    }

    /// Drop a snapshot, which is what retiring a member frees.
    pub fn discard_snapshot(&mut self, tag: &str) -> Result<()> {
        self.monitor_mut()?.delete_snapshot(tag)
    }

    fn monitor_mut(&mut self) -> Result<&mut Monitor> {
        self.monitor.as_mut().ok_or_else(|| {
            HarnessError::Configuration("the guest has no monitor".to_owned())
        })
    }

    /// Ask the emulator to stop, wait for the process to go, and remove the
    /// working directory.
    pub fn shutdown(mut self) -> Result<()> {
        let outcome = self.stop_emulator();
        self.shut_down = true;
        self.remove_work_dir();
        outcome
    }

    fn stop_emulator(&mut self) -> Result<()> {
        if let Some(monitor) = self.monitor.as_mut() {
            // A monitor that refuses `quit` is not a reason to leave a guest
            // running: the process is signalled either way.
            let _ = monitor.quit();
        }
        wait_for_exit(&mut self.child, Duration::from_secs(30))?;
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn remove_work_dir(&mut self) {
        if self.work_dir.exists() {
            let _ = fs::remove_dir_all(&self.work_dir);
        }
    }
}

impl Drop for ActiveGuest {
    /// A guest that reaches the end of its scope without a `shutdown` - a
    /// failed check, a dropped future - still leaves nothing running. The
    /// lane's own teardown is the clean path; this is the backstop that
    /// keeps a red test from becoming a leaked emulator.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn drop(&mut self) {
        if self.shut_down {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.remove_work_dir();
    }
}

/// Boot a guest and wait for it to report activation.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn boot(spec: &GuestSpec) -> Result<ActiveGuest> {
    let work_dir = spec.prepare_work_dir()?;
    // The host end of the guest's command channel, bound and listening
    // before the emulator exists: the chardev the command line declares
    // connects here, and a socket with nobody listening on it is a launch
    // that blocks at machine start rather than a guest that boots.
    let command_channel = bind_command_channel(&work_dir)?;
    let command_line = spec.command_line()?;
    let program = command_line
        .first()
        .ok_or_else(|| HarnessError::Configuration("the emulator command line is empty".to_owned()))?;
    let mut command = Command::new(program);
    command
        .args(&command_line[1..])
        // The guest's ephemeral block overlays belong to the guest that owns
        // them, not to a shared temporary directory that a concurrent guest
        // could fill.
        .env("TMPDIR", &work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // The emulator's own diagnostics are kept: an emulator that refuses
        // its command line says why on stderr, and that reason is the whole
        // difference between a wrong option and a broken host.
        .stderr(Stdio::from(fs::File::create(work_dir.join("emulator.log")).map_err(
            |error| HarnessError::io("opening the emulator log", error),
        )?));
    let mut child = command.spawn().map_err(|error| HarnessError::Spawn {
        detail: format!("{}: {error}\n{command_line:?}", program.to_string_lossy()),
    })?;

    let outcome = connect_monitor(&mut child, &work_dir, &command_line)
        .and_then(|monitor| wait_for_activation(&mut child, spec, &work_dir, monitor));
    match outcome {
        Ok(monitor) => Ok(ActiveGuest {
            child,
            monitor: Some(monitor),
            work_dir,
            command_channel: Some(command_channel),
            shut_down: false,
        }),
        Err(error) => {
            // The guest never became usable; the working directory and the
            // process go with it, and the guest's own account of the stall
            // stays in the error.
            let tail = activation_failure_tail(&read_console(&work_dir.join("console.log")));
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_dir_all(&work_dir);
            Err(decorate(error, tail))
        }
    }
}

/// Attach the guest's own account of the stall a launch failure is only
/// useful with.
fn decorate(error: HarnessError, console_tail: String) -> HarnessError {
    match error {
        HarnessError::NotActivated {
            bound,
            marker,
            console_tail: _,
        } => HarnessError::NotActivated {
            bound,
            marker,
            console_tail,
        },
        other => other,
    }
}

/// Open the monitor socket the emulator was told to create, bounded so an
/// emulator that never reaches it fails the lane instead of hanging on a
/// connect.
///
/// Every failure out of here carries the emulator's own stderr. The monitor
/// socket is created before the rest of the command line is finished being
/// acted on, so an emulator that dies on a later argument has already
/// accepted the connection and then resets it - a reset that names the
/// socket and nothing about the option that caused it. The reason is only
/// in the emulator's stderr, so it is read here and travels with the error.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn connect_monitor(
    child: &mut Child,
    work_dir: &Path,
    command_line: &[OsString],
) -> Result<Monitor> {
    let socket = work_dir.join("qmp.sock");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()
            .map_err(|error| HarnessError::io("waiting for the emulator", error))?
        {
            return Err(HarnessError::Spawn {
                detail: format!(
                    "the emulator exited with {status} before its monitor was reachable\n{}",
                    emulator_diagnostics(work_dir, command_line)
                ),
            });
        }
        if socket.exists() {
            return match Monitor::connect(&socket) {
                Ok(monitor) => Ok(monitor),
                Err(error @ (HarnessError::Monitor { .. } | HarnessError::Io { .. })) => {
                    Err(HarnessError::Spawn {
                        detail: format!(
                            "the emulator accepted its monitor connection and then stopped: {error}\n{}",
                            emulator_diagnostics(work_dir, command_line)
                        ),
                    })
                }
                Err(other) => Err(other),
            };
        }
        if Instant::now() >= deadline {
            return Err(HarnessError::Configuration(
                "the emulator did not open its monitor socket within 30s".to_owned(),
            ));
        }
        sleep(Duration::from_millis(50));
    }
}

/// What the emulator was asked to do, and what it said about it.
fn emulator_diagnostics(work_dir: &Path, command_line: &[OsString]) -> String {
    format!(
        "command line: {command_line:?}\n\
         emulator said:\n{}",
        console_tail(&work_dir.join("emulator.log"))
    )
}

/// Wait for the guest's activation contract, bounded.
///
/// The guest writes one line on its serial console once the units its own
/// configuration declares are active and its random pool is initialised.
/// Everything the lane needs to know about a boot that did not reach that
/// point is in the console it was reading: the guest reports the ordering
/// state it stalled in on the way, and that report travels with the failure
/// ahead of the console tail.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn wait_for_activation(
    child: &mut Child,
    spec: &GuestSpec,
    work_dir: &Path,
    mut monitor: Monitor,
) -> Result<Monitor> {
    let console = work_dir.join("console.log");
    let marker = spec.manifest.activation.marker.as_str();
    let deadline = Instant::now() + spec.activation_timeout;
    loop {
        if let Some(line) = activation_line(&read_console(&console), marker) {
            check_shape(&spec.manifest.activation.shape, &line)?;
            return Ok(monitor);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| HarnessError::io("waiting for the emulator", error))?
        {
            // The monitor is drained before the failure is reported so a
            // guest that stopped itself is described rather than guessed at.
            let _ = monitor.take_events();
            return Err(HarnessError::NotActivated {
                bound: spec.activation_timeout,
                marker: marker.to_owned(),
                console_tail: format!(
                    "the emulator exited with {status}\n{}",
                    activation_failure_tail(&read_console(&console))
                ),
            });
        }
        if Instant::now() >= deadline {
            return Err(HarnessError::NotActivated {
                bound: spec.activation_timeout,
                marker: marker.to_owned(),
                console_tail: activation_failure_tail(&read_console(&console)),
            });
        }
        sleep(CONSOLE_POLL);
    }
}

/// The guest's activation line, if the console has produced one.
///
/// The marker is matched anywhere in the console, not only at the start of a
/// line, because a serial console is a byte stream several writers share and
/// line structure is not a property the launcher can rely on. A login prompt
/// on the same tty emits terminal queries - cursor position, erase display -
/// and lands them mid-line with no newline of its own, which puts the guest's
/// marker in the middle of a line whose first byte is an escape sequence. The
/// guest writes its marker onto a line of its own, and the launcher does not
/// depend on that having worked.
fn activation_line(console: &str, marker: &str) -> Option<String> {
    console
        .lines()
        .find(|line| line.contains(marker))
        .map(str::to_owned)
}

/// A guest that reports activation for a shape it was not booted as is a
/// wrong image, not a slow guest, and says so.
fn check_shape(expected: &str, line: &str) -> Result<()> {
    let reported = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("shape="))
        .unwrap_or("unknown");
    if reported == expected {
        Ok(())
    } else {
        Err(HarnessError::WrongShape {
            expected: expected.to_owned(),
            reported: reported.to_owned(),
        })
    }
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_console(console: &Path) -> String {
    fs::read_to_string(console).unwrap_or_default()
}

/// The last [`CONSOLE_TAIL_BYTES`] of a console.
fn last_console_bytes(console: &str) -> String {
    if console.len() <= CONSOLE_TAIL_BYTES {
        return console.to_owned();
    }
    let mut start = console.len() - CONSOLE_TAIL_BYTES;
    while start < console.len() && !console.is_char_boundary(start) {
        start += 1;
    }
    console[start..].to_owned()
}

/// The last [`CONSOLE_TAIL_BYTES`] of the guest's console.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn console_tail(console: &Path) -> String {
    last_console_bytes(&read_console(console))
}

/// The guest's own account of the ordering state it stalled in.
///
/// The span runs from the first report the guest opened to the last one it
/// closed, so a guest that reported the stall more than once as its state
/// moved is reported whole rather than half.
fn ordering_report(console: &str) -> Option<&str> {
    let opened = console.find(ORDERING_REPORT_OPENS)?;
    let closed = console.rfind(ORDERING_REPORT_CLOSES)? + ORDERING_REPORT_CLOSES.len();
    (closed > opened).then_some(&console[opened..closed])
}

/// What an activation failure carries: the guest's ordering report first,
/// because that is the part a reader acts on, and the console tail behind
/// it, because that is the part that says where the guest got to.
fn activation_failure_tail(console: &str) -> String {
    let tail = last_console_bytes(console);
    match ordering_report(console) {
        Some(report) => {
            format!("--- guest ordering report ---\n{report}\n--- guest console tail ---\n{tail}")
        }
        None => tail,
    }
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn wait_for_exit(child: &mut Child, bound: Duration) -> Result<()> {
    let deadline = Instant::now() + bound;
    loop {
        if child
            .try_wait()
            .map_err(|error| HarnessError::io("waiting for the emulator", error))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HarnessError::Configuration(format!(
                "the emulator did not exit within {}s of being asked to",
                bound.as_secs()
            )));
        }
        sleep(Duration::from_millis(50));
    }
}

/// Reserve a loopback port and hand back its number, with nothing listening
/// on it once this returns.
///
/// This is a convenience for a caller that is about to bind that port itself,
/// so it never has to choose one and race another guest for it. The lane's
/// own guest-control channel is not one of those ports: a check's assertions
/// reach the guest over the virtio serial console the launch declared, not
/// over a forwarded TCP port, so the guest's ssh capability - which the
/// guest node declares on its own and the lane does not depend on - is
/// neither reached nor forwarded here. Nothing in the lane calls this today;
/// it is the launcher's small piece of the vocabulary a caller would use to
/// add a host-side port of its own.
pub fn reserve_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| HarnessError::io("reserving a loopback port", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| HarnessError::io("reading the reserved port", error))?
        .port();
    drop(listener);
    Ok(port)
}

/// Write a line to the lane's own report.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn report(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        Activation, Boot, Drive, Footprint, ImageFiles, Machine, PoolBudget, SharedDirectory,
    };

    fn manifest(cores: u32, memory: u32, extra: &[&str]) -> GuestManifest {
        GuestManifest {
            schema_version: 1,
            system: "x86_64-linux".to_owned(),
            node_shape: "daemon".to_owned(),
            image: ImageFiles {
                disk: "disk.qcow2".to_owned(),
                disk_format: "qcow2".to_owned(),
                disk_size_mib: 8192,
                system_image: None,
            },
            machine: Machine {
                cores,
                memory_size_mib: memory,
            },
            check: None,
            footprint: Footprint {
                memory_size_mib: u64::from(memory),
                cores,
                working_directory_mib: 8192,
            },
            pool: PoolBudget {
                memory_share_numerator: 2,
                memory_share_denominator: 3,
                core_share_numerator: 3,
                core_share_denominator: 4,
                working_directory_follows_disk: true,
            },
            boot: Boot {
                method: "direct".to_owned(),
                kernel: Some("kernel".to_owned()),
                initrd: Some("initrd".to_owned()),
                append: Some("console=ttyS0,115200n8".to_owned()),
            },
            drives: vec![Drive {
                name: Some("root".to_owned()),
                file: "disk.qcow2".to_owned(),
                format: None,
                cache: "writeback".to_owned(),
                werror: "report".to_owned(),
                boot_index: Some("1".to_owned()),
                serial: Some("root".to_owned()),
                interface: "virtio".to_owned(),
            }],
            extra_options: extra.iter().map(|value| (*value).to_owned()).collect(),
            networking_options: vec![
                "-net nic,netdev=user.0,model=virtio".to_owned(),
                "-netdev user,id=user.0,".to_owned(),
            ],
            shared_directories: vec![
                SharedDirectory {
                    mount_tag: "nix-store".to_owned(),
                    security_model: "none".to_owned(),
                    target: "/nix/.ro-store".to_owned(),
                    source: "/nix/store".to_owned(),
                },
                SharedDirectory {
                    mount_tag: "xchg".to_owned(),
                    security_model: "none".to_owned(),
                    target: "/tmp/xchg".to_owned(),
                    source: "\"$TMPDIR\"/xchg".to_owned(),
                },
                SharedDirectory {
                    mount_tag: "shared".to_owned(),
                    security_model: "none".to_owned(),
                    target: "/tmp/shared".to_owned(),
                    source: "\"${SHARED_DIR:-$TMPDIR/xchg}\"".to_owned(),
                },
            ],
            activation: Activation {
                readiness: "console-marker".to_owned(),
                contract: "d2b-daemon-acceptance".to_owned(),
                marker: "D2B_LANE_READY".to_owned(),
                serial_device: "ttyS0".to_owned(),
                acceptance_units_file: "/etc/d2b/daemon-acceptance-units".to_owned(),
                shape: "daemon".to_owned(),
            },
            init: "/nix/store/x/init".to_owned(),
            toplevel: "/nix/store/x".to_owned(),
            host_tool_bundle: "/nix/store/y".to_owned(),
            host_tool_inventory: vec!["d2b".to_owned()],
            cloud_hypervisor_controller: None,
        }
    }

    fn spec(manifest: GuestManifest) -> GuestSpec {
        GuestSpec::new(
            manifest,
            "/run/lane/image",
            "/nix/store/qemu/bin/qemu-kvm",
            "/run/lane/work",
            "lane-member-0",
        )
    }

    fn argv(spec: &GuestSpec) -> Vec<String> {
        spec.command_line()
            .expect("the command line renders")
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn value_of<'a>(argv: &'a [String], flag: &str) -> &'a str {
        let index = argv
            .iter()
            .position(|argument| argument == flag)
            .unwrap_or_else(|| panic!("{flag} is on the command line: {argv:?}"));
        &argv[index + 1]
    }

    #[test]
    fn the_declared_machine_reaches_the_command_line() {
        // A check whose configuration asks for a different memory size, vCPU
        // count, and disk gets exactly that, because the manifest is the only
        // record of the invocation and the launcher restates none of it.
        let bigger = spec(manifest(8, 16384, &[]));
        let argv = argv(&bigger);
        assert_eq!(value_of(&argv, "-m"), "16384");
        assert_eq!(value_of(&argv, "-smp"), "8");
        let drive = value_of(&argv, "-drive");
        assert!(
            drive.contains("file=/run/lane/work/lane-member-0/disk.qcow2"),
            "the root drive is the guest's own writable copy: {drive}"
        );
        assert!(!drive.contains("format="), "the node pinned no format: {drive}");
    }

    #[test]
    fn a_different_node_shape_reaches_the_command_line() {
        let mut declared = manifest(3, 3072, &[]);
        declared.machine.cores = 5;
        declared.machine.memory_size_mib = 5120;
        let argv = argv(&spec(declared));
        assert_eq!(value_of(&argv, "-smp"), "5");
        assert_eq!(value_of(&argv, "-m"), "5120");
    }

    #[test]
    fn a_per_check_device_option_reaches_the_command_line_untouched() {
        // The vsock device the guest-shell-service check declares is an
        // ordinary entry in the node's own option list, and it rides through
        // exactly as the node wrote it.
        let declared = manifest(
            3,
            3072,
            &["-drive", "file=/nix/store/z-state.img,format=raw,if=virtio", "-device", "vhost-vsock-pci,guest-cid=3"],
        );
        let argv = argv(&spec(declared));
        assert!(argv.windows(2).any(|pair| pair
            == ["-device", "vhost-vsock-pci,guest-cid=3"]));
        assert!(argv
            .iter()
            .any(|argument| argument.contains("z-state.img")));
    }

    #[test]
    fn the_networking_the_node_declared_reaches_the_command_line() {
        // The regression this unit's activation failure was. The VM module
        // renders `virtualisation.qemu.networkingOptions` in its own run
        // script as a group separate from `virtualisation.qemu.options`, and
        // a launcher that carries only the option list boots a guest with no
        // network device at all. The guest boots cleanly, the kernel never
        // loads the module that device needs, and the daemon refuses to
        // start over exactly that - with a unit that failed and a guest that
        // looks fine, which is the worst shape a boot failure can have.
        let argv = argv(&spec(manifest(3, 3072, &[])));
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["-net", "nic,netdev=user.0,model=virtio"]),
            "the guest gets the network device the node declared: {argv:?}"
        );
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["-netdev", "user,id=user.0,"]),
            "and the backend that device rides on: {argv:?}"
        );
    }

    #[test]
    fn the_networking_options_take_the_position_the_vm_module_gives_them() {
        // After the random device and before the drives, which is where the
        // VM module's own run script puts them, so a snapshot of the command
        // line reads like the invocation the node declared.
        let argv = argv(&spec(manifest(3, 3072, &[])));
        let position = |needle: &str| {
            argv.iter()
                .position(|argument| argument.starts_with(needle))
                .unwrap_or_else(|| panic!("{needle} is on the command line: {argv:?}"))
        };
        assert!(position("virtio-rng-pci") < position("-net"));
        assert!(position("-net") < position("-drive"));
    }

    #[test]
    fn one_entry_naming_a_flag_and_its_value_becomes_two_arguments() {
        // The regression this unit's boot failure was: the node declares
        // `-device virtio-keyboard` as a single list entry, the VM module's
        // run script word-splits it, and handing the entry to `execwp` whole
        // makes the emulator refuse it.
        let declared = manifest(
            3,
            3072,
            &["-device virtio-keyboard", "-usb", "-device virtio-keyboard"],
        );
        let argv = argv(&spec(declared));
        assert!(
            argv.windows(2).any(|pair| pair == ["-device", "virtio-keyboard"]),
            "the flag and its value reach the emulator as two arguments: {argv:?}"
        );
        assert!(
            !argv.iter().any(|argument| argument.contains(' ')),
            "no argument carries the node's internal whitespace: {argv:?}"
        );
    }

    #[test]
    fn an_entry_carrying_its_own_properties_reaches_the_emulator_whole() {
        let declared = manifest(
            3,
            3072,
            &["-device usb-tablet,bus=usb-bus.0", "-device vhost-vsock-pci,guest-cid=3"],
        );
        let argv = argv(&spec(declared));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["-device", "usb-tablet,bus=usb-bus.0"]));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["-device", "vhost-vsock-pci,guest-cid=3"]));
    }

    #[test]
    fn the_accelerator_is_selected_and_never_falls_back_to_emulation() {
        let argv = argv(&spec(manifest(3, 3072, &[])));
        let machine = value_of(&argv, "-machine");
        assert_eq!(machine, "accel=kvm", "no tcg fallback on this path");
        assert!(!argv.iter().any(|argument| argument.contains("tcg")));
        assert_eq!(value_of(&argv, "-cpu"), "max");
    }

    #[test]
    fn a_reusable_pool_guest_carries_an_identity_device_and_a_random_source() {
        let argv = argv(&spec(manifest(3, 3072, &[])));
        assert!(argv.windows(2).any(|pair| pair == ["-device", "vmgenid,guid=auto"]));
        assert!(argv.windows(2).any(|pair| pair == ["-device", "virtio-rng-pci"]));
    }

    #[test]
    fn the_direct_boot_shape_is_handed_its_kernel_initrd_and_command_line() {
        let argv = argv(&spec(manifest(3, 3072, &[])));
        assert_eq!(value_of(&argv, "-kernel"), "/run/lane/image/kernel");
        assert_eq!(value_of(&argv, "-initrd"), "/run/lane/image/initrd");
        assert_eq!(value_of(&argv, "-append"), "console=ttyS0,115200n8");
    }

    #[test]
    fn the_bootloader_shape_gets_no_kernel_and_keeps_its_root_drive() {
        // The writable-store shape boots its own disk through the bootloader
        // the image was built with, and its root drive is the one the node
        // declared - an unsafe cache, a boot index, and the serial the guest
        // mounts by - rather than the direct-boot shape's.
        let mut declared = manifest(3, 3072, &[]);
        declared.node_shape = "writable-store".to_owned();
        declared.boot = Boot {
            method: "bootloader".to_owned(),
            kernel: None,
            initrd: None,
            append: None,
        };
        declared.image.system_image = Some("/nix/store/z/nixos.qcow2".to_owned());
        declared.drives[0].cache = "unsafe".to_owned();
        declared.drives[0].werror = "report".to_owned();
        declared.activation.shape = "writable-store".to_owned();
        let argv = argv(&spec(declared));
        assert!(!argv.iter().any(|argument| argument == "-kernel"));
        assert!(!argv.iter().any(|argument| argument == "-append"));
        let drive = value_of(&argv, "-drive");
        assert!(drive.contains("cache=unsafe"), "{drive}");
        let device = argv
            .iter()
            .find(|argument| argument.starts_with("virtio-blk-pci,"))
            .expect("the root drive is attached");
        assert!(device.contains("bootindex=1"), "{device}");
        assert!(device.contains("serial=root"), "{device}");
    }

    #[test]
    fn a_shared_directory_relative_to_tmpdir_lands_in_the_guests_own_work_dir() {
        let argv = argv(&spec(manifest(3, 3072, &[])));
        let share = argv
            .iter()
            .find(|argument| argument.contains("mount_tag=xchg"))
            .expect("the scratch share is exported");
        assert!(
            share.contains("path=/run/lane/work/lane-member-0/xchg"),
            "{share}"
        );
        let store = argv
            .iter()
            .find(|argument| argument.contains("mount_tag=nix-store"))
            .expect("the store share is exported");
        assert!(store.contains("path=/nix/store"), "{store}");
        let shared = argv
            .iter()
            .find(|argument| argument.contains("mount_tag=shared"))
            .expect("the caller-redirectable share is exported");
        assert!(
            shared.contains("path=/run/lane/work/lane-member-0/xchg"),
            "a share the caller can redirect still names the guest's own scratch directory: {shared}"
        );
    }

    #[test]
    fn the_console_and_the_monitor_are_where_the_lane_can_reach_them() {
        let argv = argv(&spec(manifest(3, 3072, &[])));
        assert_eq!(
            value_of(&argv, "-serial"),
            "file:/run/lane/work/lane-member-0/console.log"
        );
        assert_eq!(
            value_of(&argv, "-qmp"),
            "unix:/run/lane/work/lane-member-0/qmp.sock,server=on,wait=off"
        );
    }

    #[test]
    fn the_guests_command_channel_is_declared_on_the_command_line() {
        // The guest's root shell is a unit that requires `dev-hvc0.device`,
        // and a unit whose device is absent when systemd reaches it is not
        // reliably restarted when the device appears later. A channel added
        // to a running guest therefore leaves a guest with no shell at all,
        // so the device and the socket the host listens on both belong to the
        // launch rather than to whatever drives the booted guest.
        let argv = argv(&spec(manifest(3, 3072, &[])));
        assert_eq!(
            value_of(&argv, "-chardev"),
            "socket,id=d2b-lane-console,path=/run/lane/work/lane-member-0/lane-console.sock"
        );
        let devices: Vec<&str> = argv
            .iter()
            .zip(argv.iter().skip(1))
            .filter(|(flag, _)| flag.as_str() == "-device")
            .map(|(_, value)| value.as_str())
            .collect();
        assert!(
            devices.contains(&"virtio-serial"),
            "the console's bus is attached: {devices:?}"
        );
        assert!(
            devices.contains(&"virtconsole,chardev=d2b-lane-console"),
            "the console is attached to the channel the launch declared: {devices:?}"
        );
    }

    #[test]
    fn a_console_carrying_the_marker_is_the_activation_signal() {
        let console = "[    2.113456] systemd: Reached target multi-user\n\
             D2B_LANE_READY shape=daemon units=d2bd.service d2b-broker.service\n\
             [    9.000000] sshd[1]: started\n";
        assert_eq!(
            activation_line(console, "D2B_LANE_READY").as_deref(),
            Some("D2B_LANE_READY shape=daemon units=d2bd.service d2b-broker.service")
        );
        assert_eq!(activation_line(console, "NOT_A_MARKER"), None);
    }

    #[test]
    fn a_login_prompt_sharing_the_console_does_not_hide_the_marker() {
        // The regression this unit's flaky lane run was. A getty on the same
        // serial tty emits terminal queries - cursor position, erase display -
        // and lands them mid-line with no newline of its own, so the guest's
        // marker ends up in the middle of a line that starts with an escape
        // sequence. Matching the marker only at the start of a line misses
        // that boot entirely, and the lane waits out its whole bound on a
        // guest that activated within a minute. The bytes below are what the
        // console actually held.
        let console = "d2b-lane-activation: waiting for the random pool\n\
             \u{1b}[!p\u{5b}104\\\u{1b}[0m\u{1b}[?7h\u{1b}[1G\u{1b}[0J\u{1b}[6n\u{1b}[32766;32766H\u{1b}[6nD2B_LANE_READY shape=daemon units=d2bd.service\r\n\
             <<< Welcome to NixOS 26.05pre-git (x86_64) - ttyS0 >>>\n";
        let line = activation_line(console, "D2B_LANE_READY")
            .expect("the marker is in the stream, wherever the stream put it");
        assert!(line.contains("D2B_LANE_READY"), "{line}");
        check_shape("daemon", &line).expect("and the shape still reads off the line");
        assert!(!line.starts_with("D2B_LANE_READY"), "{line}");
    }

    #[test]
    fn a_guest_that_reports_the_wrong_shape_is_a_wrong_image() {
        let error = check_shape("writable-store", "D2B_LANE_READY shape=daemon units=d2bd.service")
            .expect_err("the shape is checked");
        let rendered = error.to_string();
        assert!(rendered.contains("daemon"), "{rendered}");
        assert!(rendered.contains("writable-store"), "{rendered}");
        check_shape("daemon", "D2B_LANE_READY shape=daemon units=d2bd.service")
            .expect("the declared shape activates");
    }

    /// The console shape a guest that stalls produces: the launcher read
    /// past the boot output, the guest announced it was waiting, and then
    /// reported the ordering graph it was waiting in.
    fn stalled_console() -> String {
        "[    2.113456] systemd: Reached target multi-user\n\
         d2b-lane-activation: waiting for d2bd.service\n\
         d2b-lane-activation: ordering state for d2bd.service (not active after 91s)\n\
         === d2b units on this boot ===\n\
         d2bd.service activating start d2b daemon\n\
         --- unit: d2bd.service\n\
         ActiveState=activating\n\
         SubState=start\n\
         After=systemd-tmpfiles-setup.service network.target d2b-broker.socket d2b.slice\n\
         Wants=d2b-broker.socket systemd-tmpfiles-setup.service\n\
         --- unit: systemd-tmpfiles-setup.service\n\
         ActiveState=inactive\n\
         SubState=dead\n\
         d2b-lane-activation: end ordering state\n\
         nixos login: \n"
            .to_owned()
    }

    #[test]
    fn the_guests_own_ordering_report_travels_with_the_activation_failure() {
        // The whole point of the report: a boot that reaches the console but
        // not the marker is described by the graph the guest stalled in, and
        // that description is in the failure without another twenty-minute
        // run to reproduce it.
        let tail = activation_failure_tail(&stalled_console());
        assert!(tail.contains("guest ordering report"), "{tail}");
        assert!(tail.contains("ordering state for d2bd.service"), "{tail}");
        assert!(
            tail.contains("After=systemd-tmpfiles-setup.service"),
            "{tail}"
        );
        assert!(tail.contains("ActiveState=activating"), "{tail}");
        assert!(tail.contains("guest console tail"), "{tail}");
        assert!(tail.find("ordering state for") < tail.find("guest console tail"));
    }

    #[test]
    fn a_console_that_carries_no_ordering_report_is_just_a_console() {
        let console = "[    2.113456] systemd: Reached target multi-user\n\
             d2b-lane-activation: waiting for d2bd.service\n";
        assert_eq!(ordering_report(console), None);
        assert_eq!(activation_failure_tail(console), console);
    }

    #[test]
    fn an_ordering_report_the_guest_opened_and_never_closed_is_not_a_report() {
        // A guest killed mid-report leaves an opening delimiter and nothing
        // else. Reading a report that was never finished would present a
        // truncated ordering graph as a complete one.
        let truncated = "d2b-lane-activation: ordering state for d2bd.service\n\
                         --- unit: d2bd.service\n";
        assert_eq!(ordering_report(truncated), None);
    }

    #[test]
    fn a_guest_that_reported_the_stall_twice_is_reported_whole() {
        let console = format!(
            "{}{}",
            stalled_console(),
            stalled_console()
                .strip_prefix("[    2.113456] systemd: Reached target multi-user\n")
                .expect("the console restarts at the report")
        );
        let report = ordering_report(&console).expect("the guest reported the stall");
        assert_eq!(
            report.matches("ordering state for d2bd.service").count(),
            2,
            "{report}"
        );
    }
}
