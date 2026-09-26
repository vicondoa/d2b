//! The emulator's control monitor.
//!
//! The lane needs three things from the emulator that the console cannot
//! give it: to read the block graph the guest attached, to read the guest's
//! own snapshot list, and to ask the emulator to stop. All three are QMP
//! commands, spoken over the unix socket the launcher creates - the same
//! `qmp-socket` readiness the repository's own service-capability table
//! names.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
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
    /// The device's id on the monitor.
    pub id: String,
    /// The backing file the emulator resolved.
    pub file: String,
    /// The image format that file is in.
    pub format: String,
    /// Whether the emulator can store a snapshot inside that file.
    pub snapshottable: bool,
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
/// nothing to refuse. A read-only node is snapshottable by construction:
/// the emulator excludes it from a snapshot's device set. Every other node
/// has to be qcow2, because in the current emulator qcow2 is the only format
/// that implements the snapshot vtable, and a node that does not is one a
/// later `snapshot-save` refuses - refusing the whole save, not just that
/// node.
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
            Some(BlockDevice {
                // An entry the emulator names by qdev path rather than by
                // drive id - a device attached after the guest started, for
                // one - carries `"device": ""` rather than omitting the key,
                // so an empty id has to read as no id, or the refusal names
                // nothing.
                id: entry
                    .get("device")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .or_else(|| entry.get("qdev").and_then(Value::as_str))
                    .unwrap_or("unnamed")
                    .to_owned(),
                file: inserted
                    .get("file")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                snapshottable: read_only || format == "qcow2",
                format,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREETING: &str = r#"{"QMP": {"version": {"qemu": {"major": 10, "minor": 2, "micro": 2}}, "capabilities": []}}"#;

    /// A monitor wired to a canned emulator, so the protocol handling the
    /// lane relies on is exercised through the same code path a live monitor
    /// drives. The canned side writes the greeting and every reply up front
    /// and then drains, which is what lets a test deliver an unsolicited
    /// event - the case a request/response fake cannot express.
    fn monitor_answering(replies: &'static [&'static str]) -> Monitor {
        let (lane_end, emulator_end) = UnixStream::pair().expect("a socket pair");
        std::thread::spawn(move || {
            let mut reader = BufReader::new(emulator_end.try_clone().expect("clone"));
            let mut writer = emulator_end;
            for reply in std::iter::once(GREETING).chain(replies.iter().copied()) {
                writeln!(writer, "{reply}").expect("write a reply");
            }
            // Keep reading so the lane never writes into a closed socket.
            let mut line = String::new();
            while reader.read_line(&mut line).expect("read a request") > 0 {
                line.clear();
            }
        });
        Monitor::from_stream(lane_end).expect("the greeting and handshake succeed")
    }

    const BLOCK_REPORT: &str = concat!(
            r#"{"return": ["#,
        r#"{"device": "file-nix-store", "inserted": {"drv": "raw", "file": "/nix/store/s.img", "ro": true}},"#,
        r#"{"device": "virtio0", "inserted": {"drv": "raw", "file": "/run/lane/disk.qcow2", "ro": false}},"#,
        r#"{"device": "virtio1", "inserted": {"drv": "qcow2", "file": "/run/lane/state.qcow2", "ro": false}},"#,
        r#"{"qdev": "ide0-cd0"}"#,
        r#"]}"#
    );

    #[test]
    fn a_writable_raw_device_is_reported_as_unsnapshottable() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, BLOCK_REPORT]);
        let devices = monitor.block_devices().expect("the report parses");
        assert_eq!(
            devices
                .iter()
                .filter(|device| !device.snapshottable)
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            vec!["virtio0"],
            "a writable non-qcow2 node is the one a snapshot-save would refuse"
        );
        assert_eq!(devices.len(), 3, "a backend with no image is skipped");
        assert_eq!(devices[0].format, "raw", "a read-only backing is judged raw");
        assert!(
            devices[0].snapshottable,
            "a read-only backing is excluded from a snapshot, so it never blocks one"
        );
        assert!(devices[2].snapshottable, "the qcow2 overlay carries the snapshot");
    }

    /// The shape the real emulator reports for a device attached after the
    /// guest started: the entry has a `device` key, but it is empty, and the
    /// name lives in `qdev`. Read literally the id is the empty string, and
    /// the lane's refusal names a device with no name.
    const HOTPLUGGED_BLOCK_REPORT: &str = concat!(
            r#"{"return": ["#,
        r#"{"device": "lane_drive_0", "inserted": {"drv": "qcow2", "file": "/run/lane/disk.qcow2", "ro": false}},"#,
        r#"{"device": "", "qdev": "/machine/peripheral/lane-refusal/virtio-backend", "inserted": {"drv": "raw", "file": "/run/lane/refusal.img", "ro": false}}"#,
        r#"]}"#
    );

    #[test]
    fn a_device_named_only_by_its_qdev_path_is_named_in_the_refusal() {
        let mut monitor = monitor_answering(&[r#"{"return": {}}"#, HOTPLUGGED_BLOCK_REPORT]);
        let devices = monitor.block_devices().expect("the report parses");
        let refused = devices
            .iter()
            .find(|device| !device.snapshottable)
            .expect("a writable raw node is refused");
        assert_eq!(
            refused.id, "/machine/peripheral/lane-refusal/virtio-backend",
            "the refusal names the device by its qdev path, not by an empty id"
        );
        assert_eq!(refused.file, "/run/lane/refusal.img");
        assert_eq!(refused.format, "raw");
        assert!(
            devices[0].snapshottable,
            "the qcow2 root drive is not what failed the lane"
        );
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
