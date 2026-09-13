mod cli;
mod install;

fn main() -> std::process::ExitCode {
    if let Err(message) = kestrelsearch::telemetry::init_from_env() {
        eprintln!("[kestrel] {message}; telemetry disabled");
    }
    let result = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(cli::run()),
        Err(_) => {
            eprintln!("[kestrel] could not initialize async runtime");
            std::process::ExitCode::FAILURE
        }
    };
    if !kestrelsearch::telemetry::shutdown() {
        eprintln!("[kestrel] telemetry shutdown/delivery failed");
    }
    result
}
