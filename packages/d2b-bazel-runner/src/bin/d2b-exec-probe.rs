#[cfg(unix)]
use std::{fs, process};

const PROBE_OK: &str = "D2B-BZLEXEC-PROBE status=closed";
const PROBE_REFUSED: &str = "D2B-BZLEXEC-PROBE status=refused";

#[cfg(unix)]
fn main() {
    let entries = match fs::read_dir("/proc/self/fd") {
        Ok(entries) => entries,
        Err(_) => {
            println!("{PROBE_REFUSED}");
            process::exit(1);
        }
    };

    let mut private_descriptor_present = false;
    let mut planted_descriptor_present = false;
    for entry in entries {
        let Ok(entry) = entry else {
            println!("{PROBE_REFUSED}");
            process::exit(1);
        };
        let Ok(descriptor) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        private_descriptor_present |= (8..=10).contains(&descriptor);
        planted_descriptor_present |= (11..=12).contains(&descriptor);
    }

    if !private_descriptor_present && !planted_descriptor_present {
        println!("{PROBE_OK}");
    } else {
        println!("{PROBE_REFUSED}");
        process::exit(2);
    }
}

#[cfg(not(unix))]
fn main() {
    println!("{PROBE_REFUSED}");
}
