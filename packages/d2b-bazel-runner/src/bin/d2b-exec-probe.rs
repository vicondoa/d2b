#[cfg(unix)]
use std::{env, fs, os::unix::fs::MetadataExt, process};

#[cfg(unix)]
fn main() {
    let mut descriptors = Vec::new();
    let entries = match fs::read_dir("/proc/self/fd") {
        Ok(entries) => entries,
        Err(_) => process::exit(1),
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let descriptor = match entry.file_name().to_string_lossy().parse::<i32>() {
            Ok(descriptor) => descriptor,
            Err(_) => continue,
        };
        let metadata = match fs::metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        descriptors.push((descriptor, metadata.dev(), metadata.ino()));
    }
    descriptors.sort_unstable();

    for (descriptor, device, inode) in &descriptors {
        println!("{descriptor} {device} {inode}");
    }

    if let Some(expected) = expected_identity() {
        let provider_seen = descriptors
            .iter()
            .any(|(_, device, inode)| (*device, *inode) == expected.provider);
        let control_seen = descriptors
            .iter()
            .any(|(_, device, inode)| (*device, *inode) == expected.control);
        if provider_seen || !control_seen {
            process::exit(2);
        }
    }
}

#[cfg(unix)]
struct ExpectedIdentity {
    provider: (u64, u64),
    control: (u64, u64),
}

#[cfg(unix)]
fn expected_identity() -> Option<ExpectedIdentity> {
    let provider = (
        env::var("D2B_EXEC_PROBE_PROVIDER_DEVICE")
            .ok()?
            .parse()
            .ok()?,
        env::var("D2B_EXEC_PROBE_PROVIDER_INODE")
            .ok()?
            .parse()
            .ok()?,
    );
    let control = (
        env::var("D2B_EXEC_PROBE_CONTROL_DEVICE")
            .ok()?
            .parse()
            .ok()?,
        env::var("D2B_EXEC_PROBE_CONTROL_INODE")
            .ok()?
            .parse()
            .ok()?,
    );
    Some(ExpectedIdentity { provider, control })
}

#[cfg(not(unix))]
fn main() {}
