//! Video decoder Process construction.

use d2b_contracts_zone_session::v3::ResourceUid;

use crate::{GpuProcessSelectionError, GpuSettings, VideoWorkerSpec};

/// Build the separate video worker declaration.
pub fn build_video_worker(
    device_uid: &ResourceUid,
    settings: &GpuSettings,
) -> Result<VideoWorkerSpec, GpuProcessSelectionError> {
    VideoWorkerSpec::new(device_uid, settings)
}
