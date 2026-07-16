#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = d2b_sk_frontend::run_from_env().await {
        eprintln!("[d2b-sk-frontend] fatal: {error}");
        std::process::exit(1);
    }
}
