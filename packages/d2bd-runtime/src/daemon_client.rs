use std::fs;

use serde_json::Value;

use crate::daemon_config::{
    DaemonConfig, ServeOptions, TestClientOptions, effective_daemon_state_dir,
};
use crate::supervisor::state::{FilesystemSnapshotStore, SnapshotStore, SystemProcReader};
use crate::typed_error::TypedError;
use crate::unix_transport::{connect_seqpacket, round_trip};

pub fn run_test_client(options: TestClientOptions) -> Result<u8, TypedError> {
    let socket = connect_seqpacket(&options.socket_path)?;
    let mut exit_code = 0u8;
    for frame in &options.frame_json {
        let response = round_trip(&socket, frame)?;
        println!("{}", String::from_utf8_lossy(&response));
        if let Ok(value) = serde_json::from_slice::<Value>(&response)
            && let Some(code) = value
                .get("error")
                .and_then(|error| error.get("exitCode"))
                .and_then(Value::as_u64)
        {
            exit_code = code as u8;
        }
    }
    Ok(exit_code)
}

pub fn apply_overrides(config: &mut DaemonConfig, options: &ServeOptions) {
    if let Some(path) = &options.public_socket_path {
        config.public_socket_path = path.clone();
    }
    if let Some(path) = &options.broker_socket_path {
        config.broker_socket_path = path.clone();
    }
    if let Some(path) = &options.state_lock_path {
        config.state_lock_path = path.clone();
    }
    if let Some(path) = &options.locks_dir {
        config.locks_dir = path.clone();
    }
}

pub fn maybe_write_state_restore_report(options: &ServeOptions) -> Result<(), TypedError> {
    let Some(report_path) = options.test_state_restore_report_path.as_ref() else {
        return Ok(());
    };
    let state_dir = effective_daemon_state_dir(options);
    let store = FilesystemSnapshotStore::new(&state_dir);
    let snapshots = SnapshotStore::list(&store).map_err(|err| TypedError::InternalIo {
        context: "enumerate daemon state snapshots".to_owned(),
        detail: err.to_string(),
    })?;
    let report = crate::supervisor::state::reconcile(&snapshots, &SystemProcReader);
    let rendered = serde_json::to_vec_pretty(&report).map_err(|err| TypedError::InternalIo {
        context: "serialize daemon state report".to_owned(),
        detail: err.to_string(),
    })?;
    fs::write(report_path, rendered).map_err(|err| TypedError::InternalIo {
        context: "write daemon state report".to_owned(),
        detail: err.to_string(),
    })
}
