//! Benchmark-only observer, copied into an isolated source tree by prepare.py.
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

static START: LazyLock<Instant> = LazyLock::new(Instant::now);
static EVENTS: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());

pub fn mark(name: &'static str) {
    let seconds = START.elapsed().as_secs_f64();
    EVENTS.lock().unwrap().push((name, seconds));
}

pub fn emit() {
    eprintln!(
        "BUDGET_PROBE {}",
        serde_json::to_string(&*EVENTS.lock().unwrap()).unwrap()
    );
}

pub fn endpoint(original: &str, provider: &str) -> String {
    std::env::var("BUDGET_FIXTURE")
        .map(|base| format!("{base}/{provider}"))
        .unwrap_or_else(|_| original.to_owned())
}
