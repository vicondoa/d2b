#[tokio::main]
async fn main() {
    if let Err(error) = d2b_unsafe_local_helper::server::run().await {
        eprintln!("d2b-unsafe-local-helper: {error}");
        std::process::exit(1);
    }
}
