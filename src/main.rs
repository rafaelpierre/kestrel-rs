mod cli;
mod install;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    cli::run().await
}
