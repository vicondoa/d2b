//! Fixture for the async-discipline gate (U13, KTD9): a migrated async path
//! with only bounded tokio primitives. Nothing here blocks a runtime worker,
//! so the gate must pass it clean.

use std::future::Future;
use std::path::Path;

/// Bounded filesystem read through the async interface.
pub async fn load_config(path: &Path) -> Vec<u8> {
    tokio::fs::read(path).await.unwrap_or_default()
}

/// Async timer instead of `std::thread::sleep`.
pub async fn backoff() {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

/// Async mutex instead of `std::sync::Mutex::lock`.
pub async fn touch(shared: &tokio::sync::Mutex<u32>) {
    let _guard = shared.lock().await;
}

/// The sanctioned adapter for blocking syscalls: the closure runs on a
/// blocking thread, never on a runtime worker.
pub async fn via_spawn_blocking(path: &Path) -> Vec<u8> {
    tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .unwrap_or_default()
}

/// An async block with pure work only.
pub fn compute() -> impl Future<Output = u32> {
    async { 1 + 1 }
}