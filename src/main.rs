use shai_assistant::cli;

#[tokio::main]
async fn main() {
    cli::run().await.unwrap()
}
