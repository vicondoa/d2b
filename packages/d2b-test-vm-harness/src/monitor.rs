//! The emulator's control monitor.
//!
//! The lane needs three things from the emulator that the console cannot
//! give it: to read the block graph the guest attached, to move the guest's
//! writable disks between an external snapshot and a scratch layer, and to
//! ask the emulator to stop. All three are QMP commands, spoken over the unix
//! socket the launcher creates - the same `qmp-socket` readiness the
//! repository's own service-capability table names.
//!
//! The snapshot the pool takes is an *external* one: a fresh qcow2 overlay
//! taken with `blockdev-snapshot-sync` against the node the device is
//! writing, which leaves the previous node read-only and makes the overlay
//! the device's new top. An internal snapshot (`savevm`, or the QMP
//! `snapshot-save` behind it) is not available to this lane: the emulator
//! refuses to save a guest whose VirtFS export is mounted in the guest, and
//! every lane guest mounts one, because the guest is booted from a host store
//! path and panics at activation without it.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::error::{HarnessError, Result};

/// A connected QMP session.
pub struct Monitor {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    events: Vec<Value>,
}

/// One writable block device the guest attached, as the emulator reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    /// The device's id on the monitor, or the path it is named by instead.
    pub id: String,
    /// The device's own path in the machine's device tree, which is what
    /// detaching and re-attaching it takes.
    pub qdev: String,
    /// The block node the device is reading and writing.
    pub node: String,
    /// The backing file the emulator resolved.
    pub file: String,
    /// The image format that file is in.
    pub format: String,
    /// Whether the node is open read-only.
    pub read_only: bool,
    /// Whether a restore can be made of this device.
    pub rotatable: bool,
}

impl BlockDevice {
    /// The device as the emulator's own diagnostics name it.
    pub fn describe(&self) -> String {
        format!("{} ({} on {})", self.id, self.format, self.qdev)
    }
}
/// How far down a node's backing chain the lane walks. A guest's chain is one
/// or two layers deep - an overlay over a store path, or an overlay over an
/// overlay - and the bound is what keeps a cycle the emulator should not
/// produce from becoming a loop.
const BACKING_DEPTH: usize = 8;

/// How often the monitor polls while it waits for an event the emulator
/// delivers asynchronously.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A block node's caching mode, as the guest's configuration declared it and
/// as a restored node has to be opened with again.
///
/// A restore re-opens the frozen image as a node, and a node opened with a
/// different caching mode than it was written with is a different device as
/// far as the guest is concerned. The lane's guests declare `writeback` on the
/// disks they boot from and `unsafe` on the ephemeral ones, and both have to
/// survive a restore unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cache {
    /// The host's write cache, flushes honored.
    Writeback,
    /// The host's write cache, flushes ignored.
    Unsafe,
    /// No host write cache, flushes honored.
    None,
    /// The host's write cache, every write also written through.
    Writethrough,
}

impl Cache {
    /// The mode a guest configuration's own spelling names.
    pub fn parse(declared: &str) -> Result<Self> {
        match declared {
            "writeback" => Ok(Self::Writeback),
            "unsafe" => Ok(Self::Unsafe),
            "none" => Ok(Self::None),
            "writethrough" => Ok(Self::Writethrough),
            other => Err(HarnessError::Configuration(format!(
                "the guest declares the unknown drive cache mode {other:?}"
            ))),
        }
    }

    /// The options a block node carrying this mode is opened with.
    pub fn as_options(self) -> Value {
        let (direct, no_flush) = match self {
            Self::Writeback => (false, false),
            Self::Unsafe => (false, true),
            Self::None => (true, false),
            Self::Writethrough => (false, true),
        };
        json!({ "direct": direct, "no-flush": no_flush })
    }
}

impl Monitor {
    /// Connect to a monitor socket, read its greeting, and enter command
    /// mode.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket).map_err(|error| {
            HarnessError::io(format!("connecting to {}", socket.display()), error)
        })?;
        Self::from_stream(stream)
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn from_stream(stream: UnixStream) -> Result<Self> {
        let mut monitor = Self {
            reader: BufReader::new(
                stream
                    .try_clone()
                    .map_err(|error| HarnessError::io("cloning the monitor socket", error))?,
            ),
            writer: stream,
            events: Vec::new(),
        };
        monitor.read_message()?;
        monitor.execute("qmp_capabilities")?;
        Ok(monitor)
    }
    /// The files underneath one node, nearest first.
    ///
    /// A restore has to recognise a device the emulator named for itself -
    /// the throwaway overlay it puts over an ephemeral drive - by the store
    /// image that overlay is on, because the overlay's own name changes every
    /// time the emulator starts and the store image is what the guest's
    /// configuration declared.
    pub fn backing_files(&mut self, node: &str) -> Result<Vec<String>> {
        let report = self.execute("query-named-block-nodes")?;
        let nodes: Vec<&Value> = report
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let mut files = Vec::new();
        let mut current = Some(node.to_owned());
        let mut depth = 0;
        while let Some(name) = current {
            let Some(entry) = nodes
                .iter()
                .find(|entry| entry.get("node-name").and_then(Value::as_str) == Some(name.as_str()))
            else {
                break;
            };
            if let Some(file) = entry.get("file").and_then(Value::as_str) {
                files.push(file.to_owned());
            }
            current = entry
                .get("children")
                .and_then(Value::as_array)
                .and_then(|children| {
                    children
                        .iter()
                        .find(|child| child.get("child").and_then(Value::as_str) == Some("backing"))
                })
                .and_then(|child| child.get("node-name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            depth += 1;
            if depth > BACKING_DEPTH {
                break;
            }
        }
        Ok(files)
    }

    /// Run one QMP command and return its `return` value.
    pub fn execute(&mut self, command: &str) -> Result<Value> {
        self.execute_with(command, json!({}))
    }

    /// Run one QMP command with arguments and return its `return` value.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn execute_with(&mut self, command: &str, arguments: Value) -> Result<Value> {
        let request = json!({"execute": command, "arguments": arguments});
        let mut line = serde_json::to_string(&request)
            .map_err(|error| HarnessError::Monitor {
                command: command.to_owned(),
                detail: error.to_string(),
            })?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|error| HarnessError::io(format!("writing {command} to the monitor"), error))?;
        loop {
            let message = self.read_message()?;
            if let Some(error) = message.get("error") {
                return Err(HarnessError::Monitor {
                    command: command.to_owned(),
                    detail: error.to_string(),
                });
            }
            // The monitor interleaves asynchronous events with command
            // replies; they carry neither `return` nor `error`, so they are
            // held for the caller and the reply is what ends the loop.
            if message.get("return").is_some() {
                return Ok(message["return"].clone());
            }
            self.events.push(message);
        }
    }

    /// Ask the emulator to exit. The emulator stops the guest and closes the
    /// process; the caller waits for it.
    pub fn quit(&mut self) -> Result<()> {
        self.execute("quit").map(|_| ())
    }

    /// The guest's block graph, one entry per attached device.
    pub fn block_devices(&mut self) -> Result<Vec<BlockDevice>> {
        let report = self.execute("query-block")?;
        Ok(parse_block_devices(&report))
    }

    /// Take the pool's snapshot of one writable device: a fresh qcow2
    /// overlay, written to `overlay`, becomes the device's top, and the node
    /// it was writing goes read-only underneath it.
    ///
    /// This is the point the pool snapshots at: after activation has
    /// completed, after every attached writable device has been proven
    /// rotatable, and before any check has touched the guest. A snapshot
    /// taken later would be a snapshot of whatever the last check left
    /// behind.
    ///
    /// The snapshot is named by the node the device is writing rather than
    /// by its drive id, because the node is the thing the snapshot is taken
    /// *of*: a drive id resolves to whichever node is currently on top, and
    /// after a restore that is not the node the caller means.
    pub fn take_overlay(
        &mut self,
        device: &BlockDevice,
        overlay: &Path,
        overlay_node: &str,
    ) -> Result<()> {
        self.execute_with(
            "blockdev-snapshot-sync",
            json!({
                "node-name": device.node,
                "snapshot-file": overlay.to_string_lossy(),
                "snapshot-node-name": overlay_node,
                "format": "qcow2",
                // The overlay records its backing by absolute path, so a
                // restore can re-open the frozen image from wherever the
                // member's working directory is when it happens.
                "mode": "absolute-paths",
            }),
        )
        .map(|_| ())
    }

    /// Re-open a frozen image as a block node, so a fresh overlay can be
    /// taken over it.
    ///
    /// A restore does not keep the snapshot in the block graph: the emulator
    /// releases a node as soon as the last device drops it, and a restore
    /// drops every device. The snapshot is the *file*, and this is how it
    /// becomes a node again.
    pub fn add_frozen_image(
        &mut self,
        node: &str,
        file: &Path,
        format: &str,
        cache: Cache,
    ) -> Result<()> {
        self.execute_with(
            "blockdev-add",
            json!({
                "node-name": node,
                "driver": format,
                "file": { "driver": "file", "filename": file.to_string_lossy() },
                "cache": cache.as_options(),
                // Read-only, because nothing may write into a snapshot. A
                // restore that leaked a write into the frozen image would
                // make the *next* restore show the check before it, which is
                // exactly the failure a member reused across checks cannot
                // have.
                "read-only": true,
            }),
        )
        .map(|_| ())
    }

    /// Re-open a layer file as a node of the caller's own.
    ///
    /// The layer a snapshot writes and the node a device is attached to are
    /// two different things here. A node `blockdev-snapshot-sync` creates is
    /// dropped as soon as nothing references it, and a restore builds its
    /// chain with no device on the graph at all - so the node the device goes
    /// back on is opened by the lane over the file the snapshot wrote. The
    /// file's own header names the frozen image underneath it, so the chain
    /// is rebuilt from the file alone, and the node is writable where the
    /// snapshot's own node would have inherited the read-only image beneath
    /// it.
    pub fn add_overlay_node(&mut self, node: &str, layer: &Path, cache: Cache) -> Result<()> {
        self.execute_with(
            "blockdev-add",
            json!({
                "node-name": node,
                "driver": "qcow2",
                "file": { "driver": "file", "filename": layer.to_string_lossy() },
                "cache": cache.as_options(),
            }),
        )
        .map(|_| ())
    }

    /// Detach a device from the block graph and wait for the emulator to
    /// finish detaching it.
    ///
    /// The wait is the point: `device_del` returns as soon as the request is
    /// accepted, and the device keeps its node until the guest acknowledges
    /// the unplug. A restore that dropped the dirty layer before that
    /// happened is refused by the emulator, and one that re-attached the
    /// device first would put two devices on one disk.
    pub fn detach_device(&mut self, device: &BlockDevice, bound: Duration) -> Result<()> {
        self.execute_with("device_del", json!({ "id": device.qdev }))?;
        self.wait_for_event("DEVICE_DELETED", bound, &device.qdev)
    }

    /// Attach a device to a node, with the properties its configuration
    /// declared.
    ///
    /// The properties go in as arguments of their own rather than flattened
    /// into the driver string. `device_add` reads that string as a model name
    /// and nothing else, and refuses the whole call over a comma in it
    /// (`... is not a valid device model name`), so the spelling the launch
    /// uses on the command line is not the spelling the monitor takes.
    ///
    /// `bootindex` is the one property whose type is not a string: the
    /// emulator takes the integer the guest's configuration wrote, and
    /// answers a string with `Invalid parameter type for 'bootindex', expected:
    /// integer`.
    pub fn attach_device(
        &mut self,
        model: &str,
        node: &str,
        id: &str,
        properties: &[(String, String)],
    ) -> Result<()> {
        let mut arguments = serde_json::Map::new();
        arguments.insert("driver".to_owned(), json!(model));
        arguments.insert("drive".to_owned(), json!(node));
        arguments.insert("id".to_owned(), json!(id));
        for (key, value) in properties {
            let value = if key == "bootindex" {
                json!(value.parse::<u64>().map_err(|_| {
                    HarnessError::Configuration(format!(
                        "the guest's drive declares the boot index {value:?}, which is not a \
                         number, and the monitor takes that property as one"
                    ))
                })?)
            } else {
                json!(value)
            };
            arguments.insert(key.clone(), value);
        }
        self.execute_with("device_add", Value::Object(arguments))
            .map(|_| ())
    }

    /// Drop a block node, which is how the layer a check dirtied is freed.
    pub fn delete_node(&mut self, node: &str) -> Result<()> {
        self.execute_with("blockdev-del", json!({ "node-name": node }))
            .map(|_| ())
    }

    /// Whether the emulator still holds a node under this name.
    pub fn node_present(&mut self, node: &str) -> Result<bool> {
        let report = self.execute("query-named-block-nodes")?;
        Ok(report
            .as_array()
            .into_iter()
            .flatten()
            .any(|entry| entry.get("node-name").and_then(Value::as_str) == Some(node)))
    }

    /// Free the layer a check dirtied, if it is still in the graph.
    ///
    /// The emulator drops an unused node's whole chain as soon as the last
    /// device on it goes away, so a restore that detaches every device first
    /// usually finds the layer already gone - which is the state the restore
    /// wants, not a failure. Asking for it is what keeps the two apart: a
    /// delete that fails because the node is not there would otherwise read
    /// as a restore that could not roll back, and the writes it carried are
    /// gone either way.
    pub fn drop_node_if_present(&mut self, node: &str) -> Result<()> {
        if self.node_present(node)? {
            self.delete_node(node)?;
        }
        Ok(())
    }

    /// Restart the guest, so it runs again from the disk it is attached to.
    ///
    /// A disk-only restore replaces what is under the guest without
    /// replacing what is in it: the page cache, the mounted filesystems and
    /// every service's state would still be the ones the check before it left
    /// behind. Resetting is what makes the restored disk the state the guest
    /// is actually running on, which is the state a check's assertions were
    /// written against.
    pub fn reset_machine(&mut self) -> Result<()> {
        self.execute("system_reset").map(|_| ())
    }

    /// Wait for one asynchronous event, bounded.
    ///
    /// The monitor's socket is a blocking stream, so the wait is a poll: a
    /// command whose reply carries the event with it, and a clock. Reading
    /// the socket under a read timeout instead would leave the reader
    /// positioned mid-line, and the next command would read the tail of this
    /// one.
    fn wait_for_event(&mut self, event: &str, bound: Duration, about: &str) -> Result<()> {
        // Only an event that arrives *after* this wait began can be evidence
        // for the call that was supposed to produce it: the emulator's events
        // are a stream, and the one a previous detach left here would satisfy
        // this detach without the device having moved at all - which is how a
        // device that is still attached reaches the command after it, in the
        // shape of `blockdev-del: Node ... is in use`.
        self.events.clear();
        let deadline = Instant::now() + bound;
        loop {
            if self
                .events
                .iter()
                .any(|message| message.get("event").and_then(Value::as_str) == Some(event))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::Configuration(format!(
                    "the emulator did not report {event} for {about} within {}s",
                    bound.as_secs()
                )));
            }
            self.execute("query-status")?;
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|error| HarnessError::io("reading the emulator monitor", error))?;
        if read == 0 {
            return Err(HarnessError::Monitor {
                command: "read".to_owned(),
                detail: "the emulator closed the monitor connection".to_owned(),
            });
        }
        serde_json::from_str(line.trim()).map_err(|error| HarnessError::Monitor {
            command: "read".to_owned(),
            detail: format!("{error}: {}", line.trim()),
        })
    }

    /// Take the events the monitor delivered while a command was in flight.
    /// The lane takes them so a guest that stopped itself is noticed rather
    /// than waited on.
    pub fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }
}

/// Turn a `query-block` report into the devices the lane has to judge.
///
/// A backend with no image behind it - an empty drive, or a device whose
/// medium is not plugged - is skipped: there is nothing to snapshot and
/// nothing to refuse. A read-only node is skipped for the same reason: the
/// emulator never writes through it, so there is nothing to roll back. Every
/// other node has to be qcow2, because an external snapshot is a qcow2
/// overlay and a node that cannot be backed by one is a device a member
/// cannot be restored from.
///
/// Two paths are read out of the report because a restore needs both: the
/// node, which is what the snapshot is taken of and what has to be dropped
/// afterwards, and the device's own path, which is what detaching it takes.
/// The emulator names a device attached after the guest started by its qdev
/// path rather than by a drive id - carrying `"device": ""` rather than
/// omitting the key - so an empty id has to read as no id, or the refusal
/// names nothing.
fn parse_block_devices(report: &Value) -> Vec<BlockDevice> {
    report
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let inserted = entry.get("inserted")?;
            let read_only = inserted
                .get("ro")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let format = inserted
                .get("drv")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            if read_only {
                return None;
            }
            // The device's own path is the parent of the path the report
            // carries: the report names the block *backend*, and the backend
            // hangs off the device it feeds.
            let qdev = entry
                .get("qdev")
                .and_then(Value::as_str)
                .map(device_path)
                .unwrap_or_default();
            let node = inserted
                .get("node-name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let id = entry
                .get("device")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map_or_else(|| qdev.clone(), str::to_owned);
            Some(BlockDevice {
                qdev,
                id,
                node: node.clone(),
                file: inserted
                    .get("file")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                read_only,
                // A node the emulator named is a node a restore can name
                // too; a device that reports none cannot be detached and
                // re-attached, so it is refused rather than silently carried
                // over to the next check.
                rotatable: format == "qcow2" && !node.is_empty(),
                format,
            })
        })
        .collect()
}

/// The device's own path in the machine's device tree, given the path a block
/// report names for it.
fn device_path(reported: &str) -> String {
    match reported.rsplit_once('/') {
        Some((parent, leaf)) if leaf.ends_with("-backend") => parent.to_owned(),
        _ => reported.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const GREETING: &str = r#"{"QMP": {"version": {"qemu": {"major": 10, "minor": 2, "micro": 2}}, "capabilities": []}}"#;

    /// A monitor wired to a canned emulator, so the protocol handling the
    /// lane relies on is exercised through the same code path a live monitor
    /// drives. The canned side writes the greeting and every reply up front
    /// and then drains, which is what lets a test deliver an unsolicited
    /// event - the case a request/response fake cannot express.
    ///
    /// The requests are kept, because a restore's whole argument is in the
    /// command the lane writes: a reply the canned side sends proves nothing
    /// about which node an overlay was taken of, or how a device was
    /// re-attached.
    fn monitor_answering(replies: &'static [&'static str]) -> Wired {
        let (lane_end, emulator_end) = UnixStream::pair().expect("a socket pair");
        let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(emulator_end.try_clone().expect("clone"));
            let mut writer = emulator_end;
            for reply in std::iter::once(GREETING).chain(replies.iter().copied()) {
                writeln!(writer, "{reply}").expect("write a reply");
            }
            // Keep reading so the lane never writes into a closed socket,
            // and keep what it wrote: that is the command under test.
            let mut line = String::new();
            while reader.read_line(&mut line).expect("read a request") > 0 {
                if let Ok(request) = serde_json::from_str(line.trim()) {
                    seen.lock().expect("the request log").push(request);
                }
                line.clear();
            }
        });
        Wired {
            monitor: Monitor::from_stream(lane_end).expect("the greeting and handshake succeed"),
            requests,
        }
    }

    /// A wired monitor and the requests the canned emulator saw.
    struct Wired {
        monitor: Monitor,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    impl std::ops::Deref for Wired {
        type Target = Monitor;

        fn deref(&self) -> &Monitor {
            &self.monitor
        }
    }

    impl std::ops::DerefMut for Wired {
        fn deref_mut(&mut self) -> &mut Monitor {
            &mut self.monitor
        }
    }

    impl Wired {
        /// The last request the lane wrote, which is the command a test is
        /// asserting about.
        ///
        /// The canned side writes its replies before it reads, so the
        /// request that earned the reply being asserted on may not be logged
        /// yet; the wait is for that, not for the command.
        fn sent(&self) -> Value {
            for _ in 0..200 {
                let last = self
                    .requests
                    .lock()
                    .ok()
                    .and_then(|log| log.last().cloned())
                    .unwrap_or(Value::Null);
                if last.get("execute").is_some_and(|command| command != "qmp_capabilities") {
                    return last;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the lane wrote no command beyond the handshake");
        }
    }

    /// The report the pinned emulator produces for a booted lane guest: a
    /// read-only host share, a raw store-backed node, a qcow2 root, and the
    /// qcow2 overlay the emulator put over an ephemeral state disk.
    const BLOCK_REPORT: &str = concat!(
            r#"{"return": ["#,
        r#"{"device": "file-nix-store", "inserted": {"drv": "raw", "file": "/nix/store/s.img", "ro": true}},"#,
        r#"{"device": "virtio0", "qdev": "/machine/peripheral-anon/device[10]/virtio-backend", "inserted": {"drv": "qcow2", "file": "/run/lane/vl.ABC123", "node-name": "lane_state.overlay", "ro": false}},"#,
        r#"{"device": "lane_drive_0", "qdev": "/machine/peripheral-anon/device[4]/virtio-backend", "inserted": {"drv": "qcow2", "file": "/run/lane/disk.qcow2", "node-name": "lane_root", "ro": false}},"#,
        r#"{"device": "lane_bad", "qdev": "/machine/peripheral-anon/device[6]/virtio-backend", "inserted": {"drv": "raw", "file": "/run/lane/refusal.img", "node-name": "lane_refusal", "ro": false}},"#,
        r#"{"qdev": "ide0-cd0"}"#,
        r#"]}"#
    );

    #[test]
    fn every_writable_device_carries_the_node_and_the_path_a_restore_needs() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, BLOCK_REPORT]);
        let devices = monitor.block_devices().expect("the report parses");
        assert_eq!(
            devices.iter().map(|device| device.id.as_str()).collect::<Vec<_>>(),
            vec!["virtio0", "lane_drive_0", "lane_bad"],
            "a read-only share and a backend with no medium are both skipped"
        );
        assert_eq!(
            devices[1].node, "lane_root",
            "the snapshot is taken of the node the device is writing"
        );
        assert_eq!(
            devices[1].qdev, "/machine/peripheral-anon/device[4]",
            "the path to detach is the device's own, not the backend's"
        );
        assert!(
            devices[0].rotatable && devices[1].rotatable,
            "a named qcow2 node is one a restore can take and put back"
        );
    }

    #[test]
    fn a_writable_node_that_is_not_qcow2_is_refused_before_the_pool_is_built() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, BLOCK_REPORT]);
        let devices = monitor.block_devices().expect("the report parses");
        let refused = devices
            .iter()
            .find(|device| !device.rotatable)
            .expect("a writable raw node is refused");
        assert_eq!(refused.id, "lane_bad");
        assert_eq!(refused.file, "/run/lane/refusal.img");
        assert_eq!(refused.format, "raw");
    }

    /// The shape the real emulator reports for a device attached after the
    /// guest started: the entry has a `device` key, but it is empty, and the
    /// name lives in `qdev`. Read literally the id is the empty string, and
    /// the lane's refusal names a device with no name.
    const HOTPLUGGED_BLOCK_REPORT: &str = concat!(
            r#"{"return": ["#,
        r#"{"device": "lane_drive_0", "qdev": "/machine/peripheral-anon/device[4]/virtio-backend", "inserted": {"drv": "qcow2", "file": "/run/lane/disk.qcow2", "node-name": "lane_root", "ro": false}},"#,
        r#"{"device": "", "qdev": "/machine/peripheral/lane-refusal/virtio-backend", "inserted": {"drv": "raw", "file": "/run/lane/refusal.img", "node-name": "lane_refusal", "ro": false}}"#,
        r#"]}"#
    );

    #[test]
    fn a_device_named_only_by_its_qdev_path_is_named_in_the_refusal() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, HOTPLUGGED_BLOCK_REPORT]);
        let devices = monitor.block_devices().expect("the report parses");
        let refused = devices
            .iter()
            .find(|device| !device.rotatable)
            .expect("a writable raw node is refused");
        assert_eq!(
            refused.id, "/machine/peripheral/lane-refusal",
            "the refusal names the device by its own qdev path, not by an empty id"
        );
        assert_eq!(refused.qdev, "/machine/peripheral/lane-refusal");
        assert!(devices[0].rotatable, "the qcow2 root drive is not what failed");
    }

    #[test]
    fn an_overlay_is_taken_against_the_node_rather_than_the_drive() {
        let mut monitor = monitor_answering(&[
            r#"{"return": {}}"#,
            r#"{"return": {}}"#,
            r#"{"return": {}}"#,
        ]);
        let device = BlockDevice {
            id: "lane_drive_0".to_owned(),
            qdev: "/machine/peripheral-anon/device[4]".to_owned(),
            node: "lane_root.overlay".to_owned(),
            file: "/run/lane/lane_root.overlay.qcow2".to_owned(),
            format: "qcow2".to_owned(),
            read_only: false,
            rotatable: true,
        };
        monitor
            .take_overlay(&device, Path::new("/run/lane/lane_root.overlay-2.qcow2"), "lane_root.overlay2")
            .expect("the overlay is taken");
        let sent = monitor.sent();
        assert_eq!(sent["execute"], "blockdev-snapshot-sync");
        assert_eq!(
            sent["arguments"]["node-name"], "lane_root.overlay",
            "the overlay is taken of the node, which a drive id would not name after a restore"
        );
        assert_eq!(sent["arguments"]["format"], "qcow2");
        assert_eq!(
            sent["arguments"]["mode"], "absolute-paths",
            "the overlay has to record its backing by a path the restore can re-open"
        );
    }

    #[test]
    fn a_frozen_image_is_re_opened_read_only() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, r#"{"return": {}}"#]);
        monitor
            .add_frozen_image(
                "lane_root",
                Path::new("/run/lane/disk.qcow2"),
                "qcow2",
                Cache::Writeback,
            )
            .expect("the frozen image re-opens");
        let sent = monitor.sent();
        assert_eq!(sent["execute"], "blockdev-add");
        assert_eq!(sent["arguments"]["read-only"], true);
        assert_eq!(sent["arguments"]["cache"]["no-flush"], false);
    }

    #[test]
    fn a_cache_mode_the_guest_did_not_declare_is_refused() {
        let error = Cache::parse("writaback").expect_err("a misspelt mode is refused");
        assert!(error.to_string().contains("writaback"), "{error}");
        assert_eq!(Cache::parse("unsafe").expect("a declared mode parses"), Cache::Unsafe);
        assert_eq!(Cache::Unsafe.as_options()["no-flush"], true);
        assert_eq!(Cache::None.as_options()["direct"], true);
    }

    #[test]
    fn detaching_a_device_waits_for_the_emulator_to_finish_it() {
        let mut monitor = monitor_answering(&[
            r#"{"return": {}}"#,
            r#"{"event": "DEVICE_DELETED", "data": {"device": "/machine/peripheral-anon/device[4]"}}"#,
            r#"{"return": {"status": "running"}}"#,
        ]);
        let device = BlockDevice {
            id: "lane_drive_0".to_owned(),
            qdev: "/machine/peripheral-anon/device[4]".to_owned(),
            node: "lane_root.overlay".to_owned(),
            file: "/run/lane/lane_root.overlay.qcow2".to_owned(),
            format: "qcow2".to_owned(),
            read_only: false,
            rotatable: true,
        };
        monitor
            .detach_device(&device, Duration::from_secs(5))
            .expect("the event arrives with the next command's reply");
    }

    #[test]
    fn a_detach_the_emulator_never_reports_fails_with_the_device_named() {
        // The handshake's reply, the detach's, and one per poll the bound
        // allows before it trips.
        let mut monitor = monitor_answering(&[
            r#"{"return": {}}"#,
            r#"{"return": {}}"#,
            r#"{"return": {"status": "running"}}"#,
            r#"{"return": {"status": "running"}}"#,
            r#"{"return": {"status": "running"}}"#,
            r#"{"return": {"status": "running"}}"#,
            r#"{"return": {"status": "running"}}"#,
            r#"{"return": {"status": "running"}}"#,
        ]);
        let device = BlockDevice {
            id: "lane_drive_0".to_owned(),
            qdev: "/machine/peripheral-anon/device[4]".to_owned(),
            node: "lane_root.overlay".to_owned(),
            file: "/run/lane/lane_root.overlay.qcow2".to_owned(),
            format: "qcow2".to_owned(),
            read_only: false,
            rotatable: true,
        };
        let error = monitor
            .detach_device(&device, Duration::from_millis(300))
            .expect_err("an unplug the emulator never confirms is a failure");
        let rendered = error.to_string();
        assert!(rendered.contains("DEVICE_DELETED"), "{rendered}");
        assert!(rendered.contains("device[4]"), "{rendered}");
    }

    #[test]
    fn a_device_comes_back_with_the_properties_its_configuration_declared() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, r#"{"return": {}}"#]);
        monitor
            .attach_device(
                "virtio-blk-pci",
                "lane_root.overlay2",
                "lane_drive_0",
                &[("serial".to_owned(), "root".to_owned()), ("bootindex".to_owned(), "1".to_owned())],
            )
            .expect("the device is attached");
        let sent = monitor.sent();
        // The properties are the emulator's own arguments and not a driver
        // string: it reads that string as a model name and refuses a comma in
        // it, and it takes `bootindex` as the integer the guest's
        // configuration wrote rather than as the text of one.
        assert_eq!(sent["arguments"]["driver"], "virtio-blk-pci");
        assert_eq!(sent["arguments"]["drive"], "lane_root.overlay2");
        assert_eq!(sent["arguments"]["id"], "lane_drive_0");
        assert_eq!(sent["arguments"]["serial"], "root");
        assert_eq!(
            sent["arguments"]["bootindex"], 1,
            "the emulator answers a string with `Invalid parameter type for 'bootindex'`"
        );
    }

    #[test]
    fn a_boot_index_that_is_not_a_number_is_refused_before_the_monitor() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#]);
        let error = monitor
            .attach_device(
                "virtio-blk-pci",
                "lane_root.overlay2",
                "lane_drive_0",
                &[("bootindex".to_owned(), "first".to_owned())],
            )
            .expect_err("a boot index the emulator cannot take is a configuration failure");
        assert!(error.to_string().contains("boot index"), "{error}");
    }

    #[test]
    fn a_monitor_error_becomes_a_named_failure() {
        let mut monitor = monitor_answering(&[
            r#"{"return": {}}"#,
            r#"{"error": {"class": "GenericError", "desc": "no such command"}}"#,
        ]);
        let error = monitor
            .execute("snapshot-save")
            .expect_err("a monitor error is a failure");
        let rendered = error.to_string();
        assert!(rendered.contains("snapshot-save"), "{rendered}");
        assert!(rendered.contains("no such command"), "{rendered}");
    }

    #[test]
    fn an_event_between_a_command_and_its_reply_does_not_end_the_command() {
        // The emulator stops the guest's CPUs around a snapshot, and the
        // resulting STOP/RESUME arrive as events. Reading one as the reply
        // would leave the lane waiting on a command that already answered.
        let mut monitor = monitor_answering(&[
            r#"{"return": {}}"#,
            r#"{"event": "STOP", "data": {}}"#,
            r#"{"return": {"status": "running"}}"#,
        ]);
        let status = monitor
            .execute("query-status")
            .expect("the reply after an event is the reply");
        assert_eq!(status["status"], "running");
        let events = monitor.take_events();
        assert_eq!(events.len(), 1, "the event is held for the caller");
        assert_eq!(events[0]["event"], "STOP");
    }
}
