// Copyright 2026 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use anyhow::Context;
use argh::FromArgs;
use base::RawDescriptor;
use cros_async::Executor;

use crate::virtio::device_constants::video::VideoDeviceConfig;
use crate::virtio::vhost_user_backend::video::VideoDecoderBackend;
use crate::virtio::vhost_user_backend::BackendConnection;
use crate::virtio::vhost_user_backend::VhostUserDeviceBuilder;

#[derive(FromArgs)]
#[argh(subcommand, name = "video-decoder")]
/// Video decoder device
pub struct Options {
    #[argh(option, arg_name = "PATH", hidden_help)]
    /// deprecated - please use --socket-path instead
    socket: Option<String>,
    #[argh(option, arg_name = "PATH")]
    /// path to the vhost-user socket to bind to.
    socket_path: Option<String>,
    #[argh(option, arg_name = "FD")]
    /// file descriptor of a connected vhost-user socket.
    fd: Option<RawDescriptor>,
    #[argh(
        option,
        arg_name = "CONFIG",
        from_str_fn(video_config_from_str),
        long = "backend"
    )]
    /// video decoder backend to use (vaapi, ffmpeg).
    config: VideoDeviceConfig,
}

fn video_config_from_str(input: &str) -> Result<VideoDeviceConfig, String> {
    serde_keyvalue::from_key_values(&format!("backend={}", input)).map_err(|e| e.to_string())
}

/// Starts a vhost-user video decoder device.
pub fn run_video_device(opts: Options) -> anyhow::Result<()> {
    let ex = Executor::new().context("Failed to create executor")?;
    let video_device = Box::new(VideoDecoderBackend::new(opts.config.backend)?);

    let conn = BackendConnection::from_opts(
        opts.socket.as_deref(),
        opts.socket_path.as_deref(),
        opts.fd,
    )?;

    conn.run_device(ex, video_device)
}
