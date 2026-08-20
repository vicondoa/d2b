//! GPU and render-node Process construction.

use d2b_contracts_zone_session::v3::ResourceUid;

use crate::{GpuProcessSelectionError, GpuSettings, GpuWorkerSpec};

/// Build the signed full-GPU or render-node worker declaration.
pub fn build_gpu_worker(
    device_uid: &ResourceUid,
    settings: &GpuSettings,
) -> Result<GpuWorkerSpec, GpuProcessSelectionError> {
    GpuWorkerSpec::gpu(device_uid, settings)
}
