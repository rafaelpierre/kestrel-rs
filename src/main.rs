mod cli;
mod install;

fn main() -> std::process::ExitCode {
    if let Err(message) = kestrelsearch::telemetry::init_from_env() {
        eprintln!("[kestrel] {message}; telemetry disabled");
    }
    let result = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(async {
            let result = cli::run().await;
            let flushed =
                kestrelsearch::diagnostic_sink::flush(std::time::Duration::from_secs(1)).await;
            let stats = kestrelsearch::diagnostic_sink::stats();
            if !flushed || stats.dropped > 0 || stats.write_failures > 0 {
                eprintln!(
                    "[kestrel] local diagnostics: flushed={flushed}, dropped={}, write_failures={}",
                    stats.dropped, stats.write_failures
                );
            }
            result
        }),
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
