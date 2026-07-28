use std::collections::BTreeSet;

use redb_resource_store_spike::{
    Mutation, Store, fixture_path, put_with_backpressure_retry, synthetic_resource,
};

fn parse_usize(arguments: &[String], name: &str) -> Result<usize, String> {
    let position = arguments
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| format!("missing {name}"))?;
    arguments
        .get(position + 1)
        .ok_or_else(|| format!("missing value for {name}"))?
        .parse::<usize>()
        .map_err(|error| format!("invalid value for {name}: {error}"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let resources = parse_usize(&arguments, "--resources")?;
    let watch_count = parse_usize(&arguments, "--watches")?;
    let path = fixture_path("rss");
    let store = Store::open(&path).await?;

    for index in 0..resources {
        put_with_backpressure_retry(
            &store,
            &format!("rss-{}", index % 32),
            Mutation::create(synthetic_resource(index)),
        )
        .await?;
    }
    let revision = store.current_revision().await?;
    let mut watches = Vec::with_capacity(watch_count);
    for index in 0..watch_count {
        let resource_type =
            ["Process", "Endpoint", "Volume", "Device", "Guest", "Policy"][index % 6];
        watches.push(
            store
                .watch(revision, BTreeSet::from([resource_type.to_owned()]))
                .await?,
        );
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let file_bytes = std::fs::metadata(&path)?.len();
    println!(
        "resources={resources} watches={} revision={revision} file_bytes={file_bytes} result=READY",
        watches.len()
    );
    std::fs::remove_file(path)?;
    Ok(())
}
