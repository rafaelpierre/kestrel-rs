#!/usr/bin/env python3
"""Create a disposable instrumented tree; never edit the shipping source tree."""
import argparse
import pathlib
import shutil
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def replace(tree, file, old, new, count=1):
    path = tree / file
    text = path.read_text()
    if text.count(old) != count:
        raise ValueError(f"{file}: expected {count} occurrences of {old!r}")
    path.write_text(text.replace(old, new))


def prepare(tree):
    tree.mkdir(parents=True, exist_ok=False)
    for name in ("src", "tests", "examples"):
        shutil.copytree(ROOT / name, tree / name)
    (tree / "benchmarks/bing-fidelity").mkdir(parents=True)
    shutil.copy2(ROOT / "benchmarks/bing-fidelity/queries.json",
                 tree / "benchmarks/bing-fidelity/queries.json")
    for name in ("Cargo.toml", "Cargo.lock", "README.md"):
        shutil.copy2(ROOT / name, tree / name)
    shutil.copy2(HERE / "probe.rs", tree / "src/budget_probe.rs")
    shutil.copy2(HERE / "retained.rs", tree / "examples/budget_retained.rs")
    shutil.copy2(HERE / "initialization.rs", tree / "examples/budget_initialization.rs")
    with (tree / "src/lib.rs").open("a") as f:
        f.write("\npub mod budget_probe;\n")
    (tree / "src/main.rs").write_text('''mod cli;
mod install;
fn main() -> std::process::ExitCode {
    use kestrelsearch::budget_probe as probe;
    probe::mark("main_entry");
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    probe::mark("runtime_ready");
    let result = runtime.block_on(cli::run());
    probe::mark("handler_return");
    drop(runtime);
    probe::mark("runtime_dropped");
    probe::emit();
    result
}
''')
    def mark(name):
        return f'kestrelsearch::budget_probe::mark("{name}");'
    replace(tree, "src/cli.rs", "match Cli::parse().command {",
            f'{mark("parse_begin")} let parsed = Cli::parse(); {mark("parse_end")} match parsed.command {{')
    replace(tree, "src/cli.rs", "async fn run_search(arguments: SearchArgs) -> ExitCode {",
            'async fn run_search(arguments: SearchArgs) -> ExitCode {' + mark("handler_entry"))
    for old, name in [('let initialize_started = Instant::now();', 'client_begin'),
                      ('timings.insert("initialize".into(), elapsed_millis(initialize_started));', 'client_end'),
                      ('let provider_diagnostics = search_report.providers;', 'search_return'),
                      ('let fetch_started = Instant::now();', 'fetch_begin'),
                      ('timings.insert("fetch".into(), elapsed_millis(fetch_started));', 'fetch_end')]:
        replace(tree, "src/cli.rs", old, old + mark(name))
    replace(tree, "src/cli.rs", 'fn search_json(results: &[SearchResult], started: Instant) -> Result<String, serde_json::Error> {',
            'fn search_json(results: &[SearchResult], started: Instant) -> Result<String, serde_json::Error> {' + mark("serialize_begin"))
    replace(tree, "src/cli.rs", 'fn print_completion(command: &str, started: Instant) {',
            'fn print_completion(command: &str, started: Instant) {' + mark("output_done"))
    # Library markers share the same monotonic origin as the binary observer.
    def libmark(name):
        return f'crate::budget_probe::mark("{name}");'
    replace(tree, "src/search.rs", 'let standard = crate::http_client::standard_builder(profile, transport)',
            libmark("standard_begin") + 'let standard = crate::http_client::standard_builder(profile, transport)')
    replace(tree, "src/search.rs", 'let yahoo = engines.contains(&Engine::Yahoo).then(|| {',
            libmark("standard_end") + libmark("yahoo_begin") + 'let yahoo = engines.contains(&Engine::Yahoo).then(|| {')
    replace(tree, "src/search.rs", 'crate::benchmarking::capture_headers("search", &profile.headers());',
            libmark("yahoo_end") + 'crate::benchmarking::capture_headers("search", &profile.headers());')
    replace(tree, "src/search.rs", '.map(|budget| tokio::time::Instant::now() + budget);',
            '.map(|budget| tokio::time::Instant::now() + budget);' + libmark("deadline_start"))
    replace(tree, "src/search.rs", 'let query_outcomes = join_all(jobs).await;',
            'let query_outcomes = join_all(jobs).await;' + libmark("collector_return"))
    replace(tree, "src/search.rs", 'let cancelled = !self.completed || self.deadline;',
            libmark("provider_drop_begin") + 'let cancelled = !self.completed || self.deadline;')
    replace(tree, "src/search.rs", 'crate::benchmarking::capture_provider_lifecycle(&diagnostic, lifecycle.as_ref());',
            'crate::benchmarking::capture_provider_lifecycle(&diagnostic, lifecycle.as_ref());' + libmark("provider_drop_end"))
    for provider, url in [('bing', 'https://www.bing.com/search'), ('yahoo', 'https://search.yahoo.com/search')]:
        replace(tree, "src/search.rs", f'.get("{url}")',
                f'.get(crate::budget_probe::endpoint("{url}", "{provider}"))')
    replace(tree, "src/fetcher.rs", 'Ok(crate::http_client::standard_builder(profile, transport).build()?)',
            libmark("fetch_client_begin") + 'let client = crate::http_client::standard_builder(profile, transport).build()?;'
            + libmark("fetch_client_end") + 'Ok(client)')
    subprocess.run(["cargo", "fmt"], cwd=tree, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=pathlib.Path)
    prepare(parser.parse_args().destination.resolve())
