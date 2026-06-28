use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    config: PathBuf,
    picker: Option<PathBuf>,
    bridge_root: PathBuf,
    check_config: bool,
}

fn main() {
    if let Err(error) = run(std::env::args().skip(1)) {
        eprintln!("d2b-clipd: {error}");
        std::process::exit(2);
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let args = parse_args(args)?;
    let config_text = std::fs::read_to_string(&args.config)
        .map_err(|error| format!("failed to read config {}: {error}", args.config.display()))?;
    let _config_json: serde_json::Value = serde_json::from_str(&config_text)
        .map_err(|error| format!("invalid config JSON {}: {error}", args.config.display()))?;
    if let Some(picker) = &args.picker
        && !picker.is_absolute()
    {
        return Err(format!("--picker path must be absolute: {}", picker.display()));
    }
    if !args.bridge_root.is_absolute() {
        return Err(format!(
            "--bridge-root path must be absolute: {}",
            args.bridge_root.display()
        ));
    }
    if args.check_config {
        println!("d2b-clipd: config ok");
        return Ok(());
    }
    println!("d2b-clipd: clipboard runtime wiring is not enabled in this foundation build");
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut config = None;
    let mut picker = None;
    let mut bridge_root = None;
    let mut check_config = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                config = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--config requires a path".to_owned())?,
                ));
            }
            "--picker" => {
                picker = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--picker requires a path".to_owned())?,
                ));
            }
            "--bridge-root" => {
                bridge_root = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--bridge-root requires a path".to_owned())?,
                ));
            }
            "--check-config" => check_config = true,
            "--help" | "-h" => {
                return Err(
                    "usage: d2b-clipd --config <path> --bridge-root <path> [--picker <path>] [--check-config]"
                        .to_owned(),
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        config: config.ok_or_else(|| "--config is required".to_owned())?,
        picker,
        bridge_root: bridge_root.ok_or_else(|| "--bridge-root is required".to_owned())?,
        check_config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_required_args() {
        let args = parse_args([
            "--config".to_owned(),
            "/etc/d2b/clipboard.json".to_owned(),
            "--bridge-root".to_owned(),
            "/run/d2b/clipd".to_owned(),
            "--check-config".to_owned(),
        ])
        .expect("args");
        assert_eq!(args.config, PathBuf::from("/etc/d2b/clipboard.json"));
        assert_eq!(args.bridge_root, PathBuf::from("/run/d2b/clipd"));
        assert!(args.check_config);
    }

    #[test]
    fn rejects_unknown_args() {
        let err = parse_args(["--wat".to_owned()]).expect_err("unknown");
        assert!(err.contains("unknown argument"));
    }
}
