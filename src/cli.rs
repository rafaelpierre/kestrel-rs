use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum};
use kestrelsearch::benchmarking::{ArtifactConfig, ArtifactDiagnostics};
use kestrelsearch::config::{
    config_path, get_installations, record_installation, remove_installation,
};
use kestrelsearch::fetcher::DEFAULT_MAX_RESPONSE_BYTES;
use kestrelsearch::skill::generate_skill_md;
use kestrelsearch::{
    Engine, FetchOptions, FetchReport, KestrelClient, PageCache, SearchMode, SearchOptions,
    SearchResult, TimeFilter, pre_rank_candidates,
};

mod diagnostics;

const SKILL_NAME: &str = "kestrelsearch";

#[derive(Debug, Parser)]
#[command(
    name = "kestrel",
    version,
    about = "Kestrel Search — web search, page extraction, and relevance ranking for AI agents.",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Install this executable for the current user or all users (macOS/Linux).
    Install(crate::install::InstallArgs),
    /// Search one or more engines and return ranked results.
    Search(Box<SearchArgs>),
    /// Fetch HTML, plain text, or Markdown from a URL without searching.
    Fetch(FetchArgs),
    /// Manage the Kestrel agent skill (SKILL.md).
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
}

#[derive(Debug, clap::Args)]
struct SearchArgs {
    /// Primary search query.
    query: String,

    /// Additional query to run. Repeat for multiple queries.
    #[arg(short = 'q', long = "query", value_name = "QUERY")]
    additional_queries: Vec<String>,

    /// Search engine. Repeat to select providers to search concurrently.
    #[arg(short = 'e', long = "engine", default_values = ["duckduckgo", "bing", "yahoo", "dogpile", "ecosia", "swisscows", "yep", "qwant", "mojeek"], action = ArgAction::Append)]
    engines: Vec<Engine>,

    /// Compatibility option: fanout is the only supported search mode.
    #[arg(long)]
    mode: Option<SearchMode>,

    /// Maximum concurrent search-engine requests (1 through Tokio MAX_PERMITS).
    #[arg(long, default_value_t = 10, value_parser = concurrency_usize)]
    search_concurrency: usize,

    /// Legacy compatibility option; ignored by result-count fanout.
    #[arg(long, value_parser = positive_usize, value_name = "N")]
    provider_quorum: Option<usize>,

    /// Stop fanout after N unique candidates per query (default: 5).
    #[arg(long, value_parser = positive_usize, value_name = "N")]
    min_results: Option<usize>,

    /// First discovery budget in seconds (default: 5), including recovery I/O and request retries.
    /// Empty deadline-limited queries retry twice with +5s allowances capped at 15s.
    /// Budgets >=15s use one attempt; defaults allow up to 32s across attempts/backoff.
    /// Must round to at least 1 ns and fit a monotonic clock deadline.
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    search_budget: Option<f64>,

    /// Disable the discovery deadline and automatic discovery retries; request timeouts still apply.
    #[arg(long, conflicts_with = "search_budget")]
    no_search_budget: bool,

    /// Number of top results to return.
    #[arg(short = 'k', long, default_value_t = 5, value_parser = positive_usize, value_name = "N")]
    top_k: usize,

    /// Maximum candidates to fetch before ranking (default: checked 3 x top-k).
    #[arg(long, value_parser = positive_usize, value_name = "N")]
    fetch_candidates: Option<usize>,

    /// Minimum positive-IDF title/snippet BM25 before the fetch cap (disabled by default).
    /// Inclusive, finite and nonnegative; zero keeps zero scores. Uses tokenized query text.
    #[arg(long, value_parser = nonnegative_f64, value_name = "SCORE")]
    min_fetch_score: Option<f64>,

    /// Pre-rank titles/snippets before selecting pages to fetch.
    #[arg(long)]
    pre_rank: bool,

    /// Explicitly enable page fetching (enabled by default).
    #[arg(long, conflicts_with = "no_fetch")]
    fetch: bool,

    /// Skip page retrieval; hybrid ranks titles/snippets. Conflicts with explicit fetch-stage options.
    #[arg(long, conflicts_with_all = [
        "fetch", "rank", "fetch_candidates", "min_fetch_score", "pre_rank", "content_limit",
        "max_response_bytes", "timeout", "fetch_budget", "cache_ttl", "cache_dir",
        "cache_max_entries", "concurrency", "parse_concurrency",
    ])]
    no_fetch: bool,

    /// Explicitly enable default hybrid ranking; requires fetching and no explicit ranking policy.
    #[arg(long, conflicts_with_all = ["no_rank", "ranking_policy"])]
    rank: bool,

    /// Skip final ranking; --pre-rank can still reorder fetch candidates.
    #[arg(long, conflicts_with = "rank")]
    no_rank: bool,

    /// Final ordering (default: hybrid; body requires page fetching).
    #[arg(long, value_enum, conflicts_with = "no_rank")]
    ranking_policy: Option<kestrelsearch::ranking::RankingPolicy>,

    /// Provider region code (for example us-en or uk-en).
    #[arg(long, default_value = "", value_name = "CODE")]
    region: String,

    /// Restrict by recency: any, d, w, m, or y. Bing ignores this filter.
    #[arg(long, default_value = "any")]
    time_filter: TimeFilter,

    /// Maximum characters to extract per fetched page.
    #[arg(long, default_value_t = 2_000, value_parser = positive_usize, value_name = "CHARS")]
    content_limit: usize,

    /// Maximum decoded body bytes per page; stop at the cap and extract the prefix.
    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES, value_parser = positive_usize, value_name = "BYTES")]
    max_response_bytes: usize,

    /// HTTP timeout in seconds when fetching pages.
    /// Must round to at least 1 ns and fit a monotonic clock deadline.
    #[arg(long, default_value_t = 10.0, value_parser = positive_f64, value_name = "SECS")]
    timeout: f64,

    /// Total seconds for candidate fetches, including enabled cache reads/writes/maintenance.
    /// Eligible pages commit while other fetches run; completed text survives storage timeout.
    /// Must round to at least 1 ns and fit a monotonic clock deadline.
    #[arg(long, default_value_t = 2.0, value_parser = positive_f64, value_name = "SECS")]
    fetch_budget: f64,

    /// Cache extracted page text for this many seconds (disabled by default).
    /// Keys preserve request URL distinctions; legacy unversioned entries are misses.
    /// Must round to at least 1 ns and fit a monotonic clock deadline.
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    cache_ttl: Option<f64>,

    /// Replay and record compatible provider progress for this many seconds (opt-in).
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    recovery_ttl: Option<f64>,

    /// Independent provider-progress directory; also works with --no-fetch.
    #[arg(long, requires = "recovery_ttl", value_name = "PATH")]
    recovery_dir: Option<PathBuf>,

    /// Best-effort retained provider units (default 1000).
    #[arg(long, requires = "recovery_ttl", value_parser = positive_usize, value_name = "N")]
    recovery_max_entries: Option<usize>,

    /// Directory for incrementally committed extracted-page cache entries.
    #[arg(long, value_name = "PATH", requires = "cache_ttl")]
    cache_dir: Option<PathBuf>,

    /// Maximum extracted-page cache entries.
    #[arg(long, value_parser = positive_usize, value_name = "N", requires = "cache_ttl")]
    cache_max_entries: Option<usize>,

    /// Maximum concurrent HTTP requests when fetching pages (1 through Tokio MAX_PERMITS).
    #[arg(long, default_value_t = 10, value_parser = concurrency_usize, value_name = "N")]
    concurrency: usize,

    /// Maximum queued/running page extraction jobs (1 through Tokio MAX_PERMITS).
    #[arg(long, default_value_t = 10, value_parser = concurrency_usize, value_name = "N")]
    parse_concurrency: usize,

    /// Output format. JSON returns results, elapsed_seconds and default structured diagnostics.
    #[arg(long, default_value = "text")]
    output: Output,

    /// Omit default structured diagnostics from JSON; restores the previous envelope. No effect on text.
    #[arg(long)]
    no_diagnostics: bool,
}

impl SearchArgs {
    fn queries(&self) -> Vec<String> {
        let mut queries = vec![self.query.clone()];
        queries.extend(self.additional_queries.clone());
        if self.min_fetch_score.is_some() {
            // Match search provenance for every later stage of gated searches.
            kestrelsearch::search::normalize_queries(&queries)
        } else {
            queries
        }
    }

    fn candidate_limit(&self) -> Result<usize, &'static str> {
        if self.no_fetch {
            Ok(0)
        } else if let Some(limit) = self.fetch_candidates {
            Ok(limit)
        } else {
            self.top_k.checked_mul(3).ok_or(
                "three times --top-k exceeds the candidate-count range; lower --top-k or supply --fetch-candidates",
            )
        }
    }

    fn effective_search_budget(&self) -> Option<Duration> {
        if self.no_search_budget {
            None
        } else {
            Some(Duration::from_secs_f64(self.search_budget.unwrap_or(5.0)))
        }
    }

    fn search_options(&self) -> SearchOptions {
        SearchOptions {
            engines: self.engines.clone(),
            mode: self.mode.unwrap_or_default(),
            region: self.region.clone(),
            time_filter: self.time_filter,
            max_concurrency: self.search_concurrency,
            provider_quorum: self.provider_quorum,
            min_results: self.min_results,
            search_budget: self.effective_search_budget(),
        }
    }
}

#[derive(Debug, clap::Args)]
struct FetchArgs {
    /// Full HTTP or HTTPS URL of the page to read.
    #[arg(value_parser = page_url)]
    url: String,

    /// Maximum characters to extract from the page.
    #[arg(long, default_value_t = 20_000, value_parser = positive_usize, value_name = "CHARS")]
    content_limit: usize,

    /// Maximum decoded body bytes; stop at the cap and extract the prefix.
    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES, value_parser = positive_usize, value_name = "BYTES")]
    max_response_bytes: usize,

    /// HTTP timeout in seconds; must round to at least 1 ns and fit a monotonic clock deadline.
    #[arg(long, default_value_t = 10.0, value_parser = positive_f64, value_name = "SECS")]
    timeout: f64,

    /// Output format. JSON returns url, content, elapsed_seconds and default structured diagnostics.
    #[arg(long, default_value = "text")]
    output: Output,

    /// Omit default structured diagnostics from JSON; restores the previous envelope. No effect on text.
    #[arg(long)]
    no_diagnostics: bool,
}

fn page_url(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value).map_err(|_| "expected a full HTTP or HTTPS URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("expected a full HTTP or HTTPS URL".into());
    }
    Ok(value.to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lower")]
enum Output {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
enum SkillCommands {
    /// Install SKILL.md for Claude Code, Codex, and/or VS Code Copilot.
    Install {
        /// Target agent. If omitted, prompt interactively.
        #[arg(long)]
        agent: Option<AgentChoice>,
        /// Install in the current project or globally for the current user.
        #[arg(long)]
        scope: Option<InstallScope>,
        /// Overwrite an existing SKILL.md without prompting.
        #[arg(long)]
        force: bool,
    },
    /// Remove previously installed SKILL.md files.
    Uninstall,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[clap(rename_all = "lower")]
enum AgentChoice {
    Claude,
    Vscode,
    Codex,
    All,
    Both,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[clap(rename_all = "lower")]
enum InstallScope {
    Project,
    Global,
}

pub async fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(code as u8);
        }
    };
    match cli.command {
        Commands::Install(arguments) => match crate::install::run(arguments) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("[kestrel] {error}");
                ExitCode::FAILURE
            }
        },
        Commands::Search(arguments) => Box::pin(run_search(*arguments)).await,
        Commands::Fetch(arguments) => run_fetch(arguments).await,
        Commands::Skill { command } => match run_skill(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("[kestrel] {error}");
                ExitCode::FAILURE
            }
        },
    }
}

async fn run_fetch(arguments: FetchArgs) -> ExitCode {
    kestrelsearch::telemetry::scope_exit("kestrel.cli.fetch", async {
    let command_started = Instant::now();
    let options = FetchOptions {
        timeout: Duration::from_secs_f64(arguments.timeout),
        content_limit: arguments.content_limit,
        max_response_bytes: arguments.max_response_bytes,
        ..FetchOptions::default()
    };
    eprintln!("[kestrel] Fetching {}...", arguments.url);
    let report =
        match kestrelsearch::fetch_all_detailed(std::slice::from_ref(&arguments.url), &options)
            .await
        {
            Ok(report) => report,
            Err(error) => {
                eprintln!("[kestrel] Fetch failed: {error}");
                return ExitCode::FAILURE;
            }
        };
    let Some(content) = report.contents.first().and_then(Option::as_deref) else {
        use kestrelsearch::FetchOutcome;
        let reason = match report.pages.first().map(|page| page.outcome) {
            Some(FetchOutcome::UnsupportedContentType) => {
                "unsupported content type (expected HTML, plain text, or Markdown)"
            }
            Some(FetchOutcome::ResponseTooLarge) => "response exceeds --max-response-bytes",
            Some(FetchOutcome::RequestFailed) => "HTTP request failed or timed out",
            _ => "no extractable page text",
        };
        eprintln!("[kestrel] Fetch failed for {}: {reason}", arguments.url);
        return ExitCode::FAILURE;
    };
    if report
        .pages
        .first()
        .is_some_and(|page| page.response_bytes >= arguments.max_response_bytes)
    {
        eprintln!(
            "[kestrel] Reached --max-response-bytes; returning extracted content from the retained prefix (page may be incomplete)"
        );
    }
    let content = format!("Source: {}\n\n{content}", arguments.url);
    kestrelsearch::telemetry::payload("output.content", &content);
    match arguments.output {
        Output::Text => println!("{content}"),
        Output::Json => {
            let mut output = serde_json::json!({
                "url": arguments.url,
                "content": content,
                "elapsed_seconds": command_started.elapsed().as_secs_f64(),
            });
            if !arguments.no_diagnostics {
                output["diagnostics"] =
                    diagnostics::direct_fetch(&report, arguments.max_response_bytes);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&output)
                    .expect("strings and finite elapsed seconds serialize to JSON")
            );
        }
    }
    print_completion("Fetch", command_started);
    ExitCode::SUCCESS
    }).await
}

async fn run_search(arguments: SearchArgs) -> ExitCode {
    kestrelsearch::telemetry::scope_exit("kestrel.cli.search", async {
    let command_started = Instant::now();
    if arguments.no_fetch
        && matches!(
            arguments.ranking_policy,
            Some(kestrelsearch::ranking::RankingPolicy::Body)
        )
    {
        let mut command = Cli::command();
        command.build();
        // Clap conflicts are unconditional; this constraint depends on the policy value.
        let mut command = command
            .find_subcommand("search")
            .cloned()
            .unwrap_or(command);
        let _ = command.error(
                clap::error::ErrorKind::ArgumentConflict,
                "--ranking-policy body cannot be used with --no-fetch; remove --no-fetch or choose provider, snippet, hybrid, or rrf",
            )
            .print();
        return ExitCode::from(2);
    }
    if let Err(message) = arguments.candidate_limit() {
        let _ = Cli::command().error(clap::error::ErrorKind::ValueValidation, message).print();
        return ExitCode::from(2);
    }
    let queries = arguments.queries();
    kestrelsearch::telemetry::payload("input.queries", &queries);
    kestrelsearch::telemetry::attribute("kestrel.top_k", arguments.top_k as i64);
    kestrelsearch::telemetry::attribute("kestrel.fetch_enabled", !arguments.no_fetch);
    kestrelsearch::telemetry::attribute("kestrel.rank_enabled", !arguments.no_rank);
    let query_label = queries.join(" | ");
    let options = arguments.search_options();
    eprintln!(
        "[kestrel] Searching {} query(s) with {} ({})...",
        queries.len(),
        arguments
            .engines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        options.mode,
    );

    let mut timings = BTreeMap::new();
    let initialize_started = Instant::now();
    let client = match KestrelClient::with_engines_and_parser_capacity(&arguments.engines, arguments.parse_concurrency) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[kestrel] Failed to initialize HTTP clients: {error}");
            return ExitCode::FAILURE;
        }
    };
    let recovery = if let Some(ttl) = arguments.recovery_ttl {
        let store = arguments.recovery_dir.clone().map_or_else(kestrelsearch::SearchRecovery::default_directory, Ok)
            .and_then(|path| kestrelsearch::SearchRecovery::new(path, Duration::from_secs_f64(ttl)))
            .and_then(|store| store.with_max_entries(arguments.recovery_max_entries.unwrap_or(1000)));
        match store { Ok(store) => Some(store), Err(error) => {
            eprintln!("[kestrel] Invalid recovery configuration: {error}"); return ExitCode::FAILURE;
        } }
    } else { None };
    let client = match &recovery { Some(store) => client.with_recovery(store.clone()), None => client };
    let interrupted = async {
        if recovery.is_none() { std::future::pending::<()>().await; }
        wait_for_shutdown().await;
        if let Some(store) = &recovery { store.cancel(); }
    };
    tokio::pin!(interrupted);
    timings.insert("initialize".into(), elapsed_millis(initialize_started));
    let started = Instant::now();
    let mut search = Box::pin(client.search_many_detailed(&queries, &options));
    let search_result = tokio::select! {
        result = &mut search => result,
        () = &mut interrupted => {
            eprintln!("[kestrel] Interrupted; draining committed provider work for at most 250 ms.");
            let _ = tokio::time::timeout(Duration::from_millis(250), &mut search).await;
            return ExitCode::from(130);
        }
    };
    let search_report = match search_result {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[kestrel] Search failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let structured = (!arguments.no_diagnostics && arguments.output == Output::Json).then(|| {
        diagnostics::SearchDiagnostics::new(
            &search_report,
            &queries,
            options.min_results.unwrap_or(5),
        )
    });
    let provider_diagnostics = search_report.providers;
    let filtered: usize = provider_diagnostics
        .iter()
        .map(|provider| provider.filtered_count)
        .sum();
    if filtered > 0 {
        eprintln!("[kestrel] Excluded {filtered} result(s) by query constraints.");
    }
    let provider_cancellations = search_report.cancelled;
    let mut results = search_report.results;
    let mut candidate_counts = BTreeMap::from([("after_search".into(), results.len())]);
    timings.insert("search".into(), elapsed_millis(started));
    if provider_cancellations > 0 {
        eprintln!(
            "[kestrel] Search stopped; cancelled {provider_cancellations} unfinished provider request(s)."
        );
    }

    let artifact_config = ArtifactConfig::from_env();
    if results.is_empty() {
        kestrelsearch::telemetry::results("output", &results);
        if let Some(config) = &artifact_config {
            write_benchmark_artifact(
                config,
                &query_label,
                &results,
                &timings,
                &queries,
                &options,
                ArtifactDiagnostics {
                    providers: &provider_diagnostics,
                    provider_cancellations,
                    fetch: None,
                    candidates: &results,
                    candidate_counts: Some(&candidate_counts),
                },
            );
        }
        eprintln!("[kestrel] No results found.");
        let diagnostic = structured.map(|d| {
            d.prepare(&results, None, arguments.no_fetch, &candidate_counts, arguments.max_response_bytes)
                .finish(&results, &candidate_counts)
        });
        match arguments.output {
            Output::Json => match search_json(&results, command_started, diagnostic.as_ref()) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("[kestrel] JSON output failed: {error}");
                    return ExitCode::FAILURE;
                }
            },
            Output::Text => println!("No results found."),
        }
        print_completion("Search", command_started);
        return ExitCode::SUCCESS;
    }
    eprintln!("[kestrel] Got {} results.", results.len());

    let should_fetch = !arguments.no_fetch;
    let should_rank = !arguments.no_rank;
    let mut fetch_diagnostics = None;
    if should_fetch {
        let selection =
            match select_fetch_candidates(results, &arguments, &queries, &mut io::stderr()).await {
                Ok(selection) => selection,
                Err(error) => {
                    eprintln!("[kestrel] Candidate selection failed: {error}");
                    return ExitCode::FAILURE;
                }
            };
        results = selection.results;
        timings.extend(selection.timings);
        candidate_counts.extend(selection.counts);
        let fetch_started = Instant::now();
        let mut stderr = io::stderr();
        let mut pages = Box::pin(attach_page_content(&client, &mut results, &arguments, &mut stderr));
        let page_result = tokio::select! {
            result = &mut pages => result,
            () = &mut interrupted => {
                eprintln!("[kestrel] Interrupted; draining page work for at most 250 ms.");
                let _ = tokio::time::timeout(Duration::from_millis(250), &mut pages).await;
                return ExitCode::from(130);
            }
        };
        fetch_diagnostics = match page_result {
            Ok(report) => Some(report),
            Err(error) => { eprintln!("[kestrel] Fetch failed: {error}"); return ExitCode::FAILURE; }
        };
        timings.insert("fetch".into(), elapsed_millis(fetch_started));
    }

    candidate_counts.insert("after_candidate_selection".into(), results.len());
    candidate_counts.insert(
        "with_content".into(),
        results.iter().filter(|r| r.content.is_some()).count(),
    );
    let candidates = artifact_config.as_ref().map(|_| results.clone());
    let structured = structured.map(|d| {
        d.prepare(
            &results,
            fetch_diagnostics.as_ref(),
            arguments.no_fetch,
            &candidate_counts,
            arguments.max_response_bytes,
        )
    });
    if should_rank {
        let policy = arguments.ranking_policy.unwrap_or(kestrelsearch::ranking::RankingPolicy::Hybrid);
        eprintln!("[kestrel] Ranking with {policy:?}...");
        let rank_started = Instant::now();
        results = kestrelsearch::ranking::rank_with_policy(results, &queries, policy);
        timings.insert("rank".into(), elapsed_millis(rank_started));
    }

    candidate_counts.insert("after_ranking".into(), results.len());
    results.truncate(arguments.top_k);
    candidate_counts.insert("returned".into(), results.len());
    if let (Some(config), Some(candidates)) = (&artifact_config, &candidates) {
        write_benchmark_artifact(
            config,
            &query_label,
            &results,
            &timings,
            &queries,
            &options,
            ArtifactDiagnostics {
                providers: &provider_diagnostics,
                provider_cancellations,
                fetch: fetch_diagnostics.as_ref(),
                candidates,
                candidate_counts: Some(&candidate_counts),
            },
        );
    }
    // Release the optional full snapshot before serializing ordinary output.
    drop(candidates);
    kestrelsearch::telemetry::results("output", &results);
    eprintln!("[kestrel] Returning top {} results.", results.len());
    let diagnostic = structured.map(|d| d.finish(&results, &candidate_counts));
    match arguments.output {
        Output::Json => match search_json(&results, command_started, diagnostic.as_ref()) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("[kestrel] JSON output failed: {error}");
                return ExitCode::FAILURE;
            }
        },
        Output::Text => render_text_results(&results, &query_label),
    }
    print_completion("Search", command_started);
    ExitCode::SUCCESS
    }).await
}

// Borrow results so adding command metadata does not clone page content.
fn search_json(
    results: &[SearchResult],
    started: Instant,
    diagnostics: Option<&serde_json::Value>,
) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct SearchOutput<'a> {
        results: &'a [SearchResult],
        elapsed_seconds: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostics: Option<&'a serde_json::Value>,
    }
    serde_json::to_string_pretty(&SearchOutput {
        results,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        diagnostics,
    })
}

fn print_completion(command: &str, started: Instant) {
    eprintln!(
        "[kestrel] {command} completed in {:.3} seconds.",
        started.elapsed().as_secs_f64()
    );
}

struct CandidateSelection {
    results: Vec<SearchResult>,
    timings: BTreeMap<String, u64>,
    counts: BTreeMap<String, usize>,
}

async fn select_fetch_candidates(
    mut results: Vec<SearchResult>,
    arguments: &SearchArgs,
    queries: &[String],
    diagnostics: &mut impl Write,
) -> Result<CandidateSelection, kestrelsearch::KestrelError> {
    kestrelsearch::telemetry::scope_result("kestrel.select_candidates", async {
    kestrelsearch::telemetry::results("selection.input", &results);
    let mut timings = BTreeMap::new();
    let mut counts = BTreeMap::new();
    if let Some(minimum) = arguments.min_fetch_score {
        let started = Instant::now();
        let queries = queries.to_vec();
        let context = kestrelsearch::telemetry::parent_context();
        let (filtered, report) = tokio::task::spawn_blocking(move || {
            let _context = context.attach();
            kestrelsearch::ranking::filter_fetch_candidates(&mut results, &queries, minimum)
                .map(|report| (results, report))
        })
        .await
        .map_err(|error| {
            kestrelsearch::KestrelError::Search(format!("fetch score task failed: {error}"))
        })??;
        results = filtered;
        timings.insert("fetch_score".into(), elapsed_millis(started));
        counts.insert("after_fetch_score".into(), results.len());
        counts.insert("fetch_score_rejected".into(), report.rejected);
        counts.insert(
            "fetch_score_bypassed_queries".into(),
            report.bypassed_queries,
        );
        let _ = writeln!(
            diagnostics,
            "[kestrel] Fetch score threshold excluded {} candidate(s).",
            report.rejected
        );
        if report.bypassed_queries > 0 {
            let _ = writeln!(
                diagnostics,
                "[kestrel] Fetch score threshold bypassed for {} query(s) without lexical terms.",
                report.bypassed_queries
            );
        }
    }
    let limit = arguments
        .candidate_limit()
        .map_err(|message| kestrelsearch::KestrelError::InvalidRequest(message.into()))?;
    if arguments.pre_rank && results.len() > limit {
        let _ = writeln!(
            diagnostics,
            "[kestrel] Pre-ranking candidates from titles and snippets..."
        );
        let started = Instant::now();
        results = pre_rank_candidates(results, queries);
        timings.insert("pre_rank".into(), elapsed_millis(started));
    }
    if results.len() > limit {
        let _ = writeln!(
            diagnostics,
            "[kestrel] Fetching the first {limit} candidates before ranking (from {} search results).",
            results.len()
        );
        results.truncate(limit);
    }
    kestrelsearch::telemetry::results("selection.output", &results);
    Ok(CandidateSelection {
        results,
        timings,
        counts,
    })
    }).await
}

async fn attach_page_content(
    client: &KestrelClient,
    results: &mut [SearchResult],
    arguments: &SearchArgs,
    diagnostics: &mut impl Write,
) -> Result<FetchReport, kestrelsearch::search::KestrelError> {
    kestrelsearch::telemetry::scope_result("kestrel.attach_content", async {
    // An all-rejected pool must not initialize or touch a page cache.
    if results.is_empty() {
        return Ok(FetchReport {
            contents: Vec::new(),
            pages: Vec::new(),
            budget_exhausted: false,
            cancelled: 0,
            cache_hits: 0,
            cache_misses: 0,
        });
    }
    let fetchable: Vec<(usize, String)> = results
        .iter()
        .enumerate()
        .filter(|(_, result)| !result.url.to_ascii_lowercase().contains(".pdf"))
        .map(|(index, result)| (index, result.url.clone()))
        .collect();
    let _ = writeln!(
        diagnostics,
        "[kestrel] Fetching {} pages (concurrency={})...",
        fetchable.len(),
        arguments.concurrency
    );
    let options = FetchOptions {
        timeout: Duration::from_secs_f64(arguments.timeout),
        content_limit: arguments.content_limit,
        max_concurrency: arguments.concurrency,
        parse_concurrency: arguments.parse_concurrency,
        max_response_bytes: arguments.max_response_bytes,
    };
    let urls: Vec<String> = fetchable.iter().map(|(_, url)| url.clone()).collect();
    let budget = Some(Duration::from_secs_f64(arguments.fetch_budget));
    let mut report = if let Some(ttl) = arguments.cache_ttl {
        let directory = arguments
            .cache_dir
            .clone()
            .map_or_else(PageCache::default_directory, Ok)?;
        let cache = PageCache::new(directory, Duration::from_secs_f64(ttl))?
            .with_max_entries(arguments.cache_max_entries.unwrap_or(1_000))?;
        client
            .fetch_all_cached_detailed(&urls, &options, &cache, budget)
            .await?
    } else {
        client.fetch_all_detailed(&urls, &options, budget).await?
    };
    write_fetch_budget_notice(&report, arguments.fetch_budget, diagnostics);
    let capped_count = report
        .pages
        .iter()
        .filter(|page| {
            page.outcome == kestrelsearch::FetchOutcome::Success
                && page.response_bytes >= options.max_response_bytes
        })
        .count();
    if capped_count > 0 {
        let _ = writeln!(
            diagnostics,
            "[kestrel] {capped_count} fetched page(s) reached --max-response-bytes; search results may contain incomplete page content."
        );
    }
    let fetched_count = report
        .contents
        .iter()
        .filter(|content| content.is_some())
        .count();
    for ((index, url), content) in fetchable.into_iter().zip(&mut report.contents) {
        let content = content.take();
        results[index].content = content.map(|content| format!("Source: {url}\n\n{content}"));
    }
    let _ = writeln!(
        diagnostics,
        "[kestrel] Successfully fetched {fetched_count}/{} pages.",
        urls.len()
    );
    Ok(report)
    }).await
}

fn write_fetch_budget_notice(
    report: &kestrelsearch::FetchReport,
    budget_seconds: f64,
    diagnostics: &mut impl Write,
) {
    if report.budget_exhausted && report.cancelled > 0 {
        let _ = writeln!(
            diagnostics,
            "[kestrel] Fetch budget exhausted ({budget_seconds}s); cancelled {} unfinished page fetch(es). Completed content was retained. Fetch individual pages with `kestrel fetch \"URL\"`, or allow more time with `--fetch-budget SECS`.",
            report.cancelled
        );
    }
}

fn render_text_results(results: &[SearchResult], query: &str) {
    println!("\n{}", "=".repeat(80));
    println!("Top {} results for: '{query}'", results.len());
    println!("{}\n", "=".repeat(80));
    for (index, result) in results.iter().enumerate() {
        let score = result
            .bm25_score
            .map(|score| format!("  [BM25: {score:.2}]"))
            .unwrap_or_default();
        let source = result
            .engine
            .zip(result.query.as_deref())
            .map(|(engine, query)| format!("  [{engine}: {query}]"))
            .unwrap_or_default();
        println!("{}. {}{score}{source}", index + 1, result.title);
        println!("   {}", result.url);
        println!("   {}", result.snippet);
        if let Some(content) = &result.content {
            println!("\n   {content}\n");
        } else {
            println!();
        }
    }
}

fn write_benchmark_artifact(
    config: &ArtifactConfig,
    query: &str,
    results: &[SearchResult],
    timings: &BTreeMap<String, u64>,
    queries: &[String],
    options: &SearchOptions,
    diagnostics: ArtifactDiagnostics<'_>,
) {
    if let Err(error) = config.write(
        query,
        results,
        timings,
        queries,
        &options.engines,
        options.mode,
        diagnostics,
    ) {
        eprintln!("[kestrel] Failed to write benchmark artifact: {error}");
    }
}

fn run_skill(command: SkillCommands) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        SkillCommands::Install {
            agent,
            scope,
            force,
        } => {
            let agent = agent.map_or_else(prompt_agent, Ok)?;
            let scope = scope.map_or_else(prompt_scope, Ok)?;
            install_skill(agent, scope, force)
        }
        SkillCommands::Uninstall => uninstall_skill(),
    }
}

fn install_skill(
    agent: AgentChoice,
    scope: InstallScope,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let targets = skill_targets(agent, scope)?;
    println!("\nWill write skill to:");
    for target in &targets {
        println!("  {}", target.display());
    }
    println!();
    let mut command = Cli::command();
    let content = generate_skill_md(&mut command);
    for target in targets {
        if target.exists()
            && !force
            && !confirm(
                &format!("{} already exists. Overwrite?", target.display()),
                false,
            )?
        {
            println!("  Skipped {}", target.display());
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &content)?;
        record_installation(&target)?;
        println!("  Installed: {}", target.display());
    }
    println!(
        "\nInstallation paths recorded in {}",
        config_path()?.display()
    );
    println!("Restart your agent session to pick up the new skill.");
    Ok(())
}

fn uninstall_skill() -> Result<(), Box<dyn std::error::Error>> {
    let installations = get_installations()?;
    if installations.is_empty() {
        println!("No skill installations recorded in config.");
        println!("(config: {})", config_path()?.display());
        return Ok(());
    }
    let mut existing = Vec::new();
    let stale: Vec<_> = installations
        .into_iter()
        .filter(|path| {
            if path.exists() {
                existing.push(path.clone());
                false
            } else {
                true
            }
        })
        .collect();
    if !stale.is_empty() {
        println!("\nThe following recorded paths no longer exist on disk (will be cleaned up):");
        for path in stale {
            println!("  {}", path.display());
            remove_installation(&path)?;
        }
    }
    if existing.is_empty() {
        println!("\nNo skill files found on disk. Config has been cleaned up.");
        return Ok(());
    }
    println!("\nInstalled skill locations:");
    for (index, path) in existing.iter().enumerate() {
        println!("  [{}] {}", index + 1, path.display());
    }
    let raw = prompt(
        "Which installation(s) to remove? (comma-separated numbers, or 'all')",
        "all",
    )?;
    let selected = select_installations(&raw, &existing);
    if selected.is_empty() {
        println!("Nothing selected. Aborting.");
        return Ok(());
    }
    println!();
    for target in selected {
        match fs::remove_file(target) {
            Ok(()) => {
                if let Some(parent) = target.parent() {
                    let _ = fs::remove_dir(parent);
                }
                remove_installation(target)?;
                println!("  Removed: {}", target.display());
            }
            Err(error) => println!("  Failed to remove {}: {error}", target.display()),
        }
    }
    println!("\nDone. Restart your agent session for changes to take effect.");
    Ok(())
}

fn skill_targets(agent: AgentChoice, scope: InstallScope) -> Result<Vec<PathBuf>, io::Error> {
    let agents: &[AgentChoice] = match agent {
        AgentChoice::All => &[AgentChoice::Claude, AgentChoice::Vscode, AgentChoice::Codex],
        AgentChoice::Both => &[AgentChoice::Claude, AgentChoice::Vscode],
        _ => std::slice::from_ref(&agent),
    };
    let home = home::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))?;
    Ok(agents
        .iter()
        .map(|agent| match (scope, agent) {
            (InstallScope::Project, AgentChoice::Claude) => PathBuf::from(".claude/skills"),
            (InstallScope::Project, AgentChoice::Vscode) => PathBuf::from(".github/skills"),
            (InstallScope::Project, AgentChoice::Codex) => PathBuf::from(".codex/skills"),
            (InstallScope::Global, AgentChoice::Claude) => home.join(".claude/skills"),
            (InstallScope::Global, AgentChoice::Vscode) => home.join(".copilot/skills"),
            (InstallScope::Global, AgentChoice::Codex) => home.join(".codex/skills"),
            (_, AgentChoice::All | AgentChoice::Both) => unreachable!("agent groups expanded"),
        })
        .map(|base| base.join(SKILL_NAME).join("SKILL.md"))
        .collect())
}

fn select_installations<'a>(raw: &str, existing: &'a [PathBuf]) -> Vec<&'a PathBuf> {
    if raw.trim().eq_ignore_ascii_case("all") {
        return existing.iter().collect();
    }
    let mut selected = Vec::new();
    for entry in raw.split(',').map(str::trim) {
        match entry.parse::<usize>() {
            Ok(index) if (1..=existing.len()).contains(&index) => {
                let path = &existing[index - 1];
                if !selected.contains(&path) {
                    selected.push(path);
                }
            }
            Ok(_) => println!("  Index {entry} out of range, skipping."),
            Err(_) => println!("  Skipping invalid entry: '{entry}'"),
        }
    }
    selected
}

fn prompt_agent() -> Result<AgentChoice, Box<dyn std::error::Error>> {
    loop {
        let value = prompt("Which agent? [claude/vscode/codex/all/both]", "all")?;
        if let Ok(agent) = AgentChoice::from_str(&value, true) {
            return Ok(agent);
        }
        println!("Invalid choice: {value}");
    }
}

fn prompt_scope() -> Result<InstallScope, Box<dyn std::error::Error>> {
    loop {
        let value = prompt("Install scope [project/global]", "project")?;
        if let Ok(scope) = InstallScope::from_str(&value, true) {
            return Ok(scope);
        }
        println!("Invalid choice: {value}");
    }
}

fn prompt(message: &str, default: &str) -> io::Result<String> {
    print!("{message} ({default}): ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn confirm(message: &str, default: bool) -> io::Result<bool> {
    let default_label = if default { "Y/n" } else { "y/N" };
    let answer = prompt(&format!("{message} [{default_label}]"), "")?;
    if answer.is_empty() {
        Ok(default)
    } else {
        Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn positive_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|error| error.to_string())
        .and_then(|value| {
            (value >= 1)
                .then_some(value)
                .ok_or_else(|| "must be at least 1".into())
        })
}

fn concurrency_usize(value: &str) -> Result<usize, String> {
    let value = positive_usize(value)?;
    if value > tokio::sync::Semaphore::MAX_PERMITS {
        return Err(format!(
            "must be at most {}",
            tokio::sync::Semaphore::MAX_PERMITS
        ));
    }
    Ok(value)
}

fn nonnegative_f64(value: &str) -> Result<f64, String> {
    let value: f64 = value.parse().map_err(|_| "must be a number".to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("must be finite and nonnegative".into());
    }
    Ok(value)
}

fn positive_f64(value: &str) -> Result<f64, String> {
    let value = value.parse::<f64>().map_err(|error| error.to_string())?;
    let duration = Duration::try_from_secs_f64(value)
        .map_err(|_| "must be finite, positive, and representable as a duration".to_string())?;
    if duration.is_zero() || Instant::now().checked_add(duration).is_none() {
        return Err(
            "must round to at least one nanosecond and fit a monotonic clock deadline".into(),
        );
    }
    // All subsequent from_secs_f64 conversions use this validated, unchanged value.
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_fetch_budget_defaults_overrides_and_notice() {
        for (extra, expected) in [
            (vec![], 2.0),
            (vec!["--no-fetch"], 2.0),
            (vec!["--fetch-budget", "0.5"], 0.5),
            (vec!["--fetch-budget", "10"], 10.0),
        ] {
            let Commands::Search(args) =
                Cli::try_parse_from(["kestrel", "search", "test"].into_iter().chain(extra))
                    .unwrap()
                    .command
            else {
                panic!("expected search")
            };
            assert_eq!(args.fetch_budget, expected);
        }
        let Commands::Fetch(args) =
            Cli::try_parse_from(["kestrel", "fetch", "https://example.org"])
                .unwrap()
                .command
        else {
            panic!("expected fetch")
        };
        assert_eq!(args.timeout, 10.0);
        for (exhausted, cancelled, notice) in [
            (false, 0, false),
            (true, 0, false),
            (false, 1, false),
            (true, 1, true),
        ] {
            let report = kestrelsearch::FetchReport {
                contents: vec![],
                pages: vec![],
                budget_exhausted: exhausted,
                cancelled,
                cache_hits: 0,
                cache_misses: 0,
            };
            let mut output = Vec::new();
            write_fetch_budget_notice(&report, 2.0, &mut output);
            let output = String::from_utf8(output).unwrap();
            assert_eq!(!output.is_empty(), notice);
            if notice {
                assert!(output.contains("cancelled 1 unfinished page fetch(es)"));
                assert!(output.contains("kestrel fetch \"URL\""));
            }
        }
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("two seconds by default"));
        assert!(skill.contains("kestrel fetch \"URL\""));
        assert!(!skill.contains("it is unset by default"));
        assert!(!skill.contains("No total fetch deadline applies when"));
    }

    #[test]
    fn numeric_boundaries_and_candidate_defaults() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for value in [
            "0",
            "NaN",
            "inf",
            "-inf",
            "1e-100",
            "1e100",
            "18446744073709551615",
        ] {
            assert!(positive_f64(value).is_err(), "{value}");
        }
        for value in ["0.000000001", "0.5", "10"] {
            assert!(positive_f64(value).is_ok(), "{value}");
        }
        let maximum = tokio::sync::Semaphore::MAX_PERMITS;
        assert!(concurrency_usize(&maximum.to_string()).is_ok());
        for value in [0, maximum + 1, usize::MAX] {
            assert!(concurrency_usize(&value.to_string()).is_err());
        }
        let Commands::Search(mut args) = Cli::try_parse_from(["kestrel", "search", "test"])
            .unwrap()
            .command
        else {
            panic!("expected search");
        };
        args.top_k = usize::MAX / 3;
        assert_eq!(args.candidate_limit().unwrap(), (usize::MAX / 3) * 3);
        args.top_k += 1;
        assert!(args.candidate_limit().is_err());
        args.top_k = usize::MAX;
        args.fetch_candidates = Some(1);
        assert_eq!(args.candidate_limit().unwrap(), 1);
        args.fetch_candidates = None;
        args.no_fetch = true;
        assert_eq!(args.candidate_limit().unwrap(), 0);
    }

    #[test]
    fn compatible_stage_options_remain_accepted() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        Cli::command().debug_assert();
        for flags in [
            vec![],
            vec!["--fetch", "--rank"],
            vec!["--no-fetch", "--no-rank"],
            vec!["--no-fetch", "--ranking-policy", "provider"],
            vec!["--no-fetch", "--ranking-policy", "snippet"],
            vec!["--no-fetch", "--ranking-policy", "hybrid"],
            vec!["--no-fetch", "--ranking-policy", "rrf"],
            vec!["--pre-rank", "--no-rank"],
            vec!["--min-fetch-score", "0", "--no-rank"],
            vec!["--min-fetch-score", "1e100", "--ranking-policy", "body"],
            vec![
                "--min-fetch-score",
                "0.1",
                "--pre-rank",
                "--ranking-policy",
                "hybrid",
            ],
            vec!["--ranking-policy", "body"],
            vec!["--fetch", "--ranking-policy", "body"],
            vec![
                "--no-fetch",
                "--search-concurrency",
                "2",
                "--search-budget",
                "1",
            ],
            vec![
                "--timeout",
                "2",
                "--fetch-budget",
                "1",
                "--search-budget",
                "3",
            ],
            vec![
                "--cache-ttl",
                "60",
                "--cache-dir",
                "cache",
                "--cache-max-entries",
                "2",
            ],
        ] {
            Cli::try_parse_from(
                ["kestrel", "search", "test"]
                    .into_iter()
                    .chain(flags.clone()),
            )
            .unwrap_or_else(|error| panic!("{flags:?}: {error}"));
        }
    }

    #[test]
    fn shell_quoting_only_groups_the_query_argument() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for (command, expected) in [
            (r#"kestrel search "machine learning""#, "machine learning"),
            (
                r#"kestrel search '"machine learning"'"#,
                r#""machine learning""#,
            ),
            (
                r#"kestrel search "filetype:pdf a OR b""#,
                "filetype:pdf a OR b",
            ),
        ] {
            let argv = shlex::split(command).unwrap();
            let Commands::Search(args) = Cli::try_parse_from(argv).unwrap().command else {
                panic!("expected search");
            };
            assert_eq!(args.queries(), [expected]);
        }
    }

    #[test]
    fn search_budget_defaults_and_overrides() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for (extra, expected) in [
            (vec![], Some(5.0)),
            (vec!["--mode", "fanout"], Some(5.0)),
            (
                vec!["--mode", "fanout", "--search-budget", "12.5"],
                Some(12.5),
            ),
            (vec!["--search-budget", "2"], Some(2.0)),
            (vec!["--mode", "fanout", "--no-search-budget"], None),
            (vec!["--no-search-budget"], None),
        ] {
            let cli = Cli::try_parse_from(["kestrel", "search", "test"].into_iter().chain(extra))
                .unwrap();
            let Commands::Search(args) = cli.command else {
                panic!("expected search");
            };
            assert_eq!(
                args.effective_search_budget(),
                expected.map(Duration::from_secs_f64)
            );
            assert_eq!(args.search_concurrency, 10);
            assert_eq!(args.concurrency, 10);
            assert_eq!(args.parse_concurrency, 10);
            assert_eq!(
                args.search_concurrency,
                SearchOptions::default().max_concurrency
            );
            assert_eq!(args.concurrency, FetchOptions::default().max_concurrency);
            assert_eq!(
                args.parse_concurrency,
                FetchOptions::default().parse_concurrency
            );
        }
        assert!(SearchOptions::default().search_budget.is_none());
        assert!(
            Cli::try_parse_from([
                "kestrel",
                "search",
                "test",
                "--search-budget",
                "5",
                "--no-search-budget"
            ])
            .is_err()
        );
    }

    #[test]
    fn search_defaults_match_library_and_explicit_engines_replace_defaults() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for extra in [vec![], vec!["--mode", "fanout"]] {
            let cli = Cli::try_parse_from(["kestrel", "search", "test"].into_iter().chain(extra))
                .unwrap();
            let Commands::Search(args) = cli.command else {
                panic!("expected search");
            };
            assert_eq!(args.engines, SearchOptions::default().engines);
            assert_eq!(
                args.engines
                    .iter()
                    .map(|engine| engine.as_str())
                    .collect::<Vec<_>>(),
                "duckduckgo bing yahoo dogpile ecosia swisscows yep qwant mojeek"
                    .split_whitespace()
                    .collect::<Vec<_>>()
            );
            assert_eq!(args.search_options().mode, SearchMode::Fanout);
            assert_eq!(args.search_options().mode, SearchOptions::default().mode);
        }
        let cli = Cli::try_parse_from([
            "kestrel", "search", "test", "--engine", "yahoo", "--engine", "bing",
        ])
        .unwrap();
        let Commands::Search(args) = cli.command else {
            panic!("expected search");
        };
        assert_eq!(args.engines, [Engine::Yahoo, Engine::Bing]);
    }

    #[test]
    fn budgeted_search_defaults_and_explicit_overrides() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for (flags, mode, quorum) in [
            (vec![], SearchMode::Fanout, None),
            (vec!["--search-budget", "3"], SearchMode::Fanout, None),
            (vec!["--search-budget", "0.5"], SearchMode::Fanout, None),
            (
                vec!["--search-budget", "3", "--provider-quorum", "2"],
                SearchMode::Fanout,
                Some(2),
            ),
            (
                vec!["--search-budget", "3", "--mode", "fanout"],
                SearchMode::Fanout,
                None,
            ),
            (vec!["--mode", "fanout"], SearchMode::Fanout, None),
            (
                vec![
                    "--mode",
                    "fanout",
                    "--search-budget",
                    "3",
                    "--provider-quorum",
                    "2",
                ],
                SearchMode::Fanout,
                Some(2),
            ),
            (
                vec!["--engine", "bing", "--search-budget", "3"],
                SearchMode::Fanout,
                None,
            ),
        ] {
            let cli = Cli::try_parse_from(
                ["kestrel", "search", "test"]
                    .into_iter()
                    .chain(flags.clone()),
            )
            .unwrap();
            let Commands::Search(args) = cli.command else {
                panic!("expected search")
            };
            let options = args.search_options();
            assert_eq!(options.mode, mode, "{flags:?}");
            assert_eq!(options.provider_quorum, quorum, "{flags:?}");
            assert_eq!(options.engines, args.engines);
            assert_eq!(
                options.search_budget,
                Some(Duration::from_secs_f64(args.search_budget.unwrap_or(5.0)))
            );
        }
    }

    #[test]
    fn fallback_mode_is_rejected() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let error =
            Cli::try_parse_from(["kestrel", "search", "test", "--mode", "fallback"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert!(error.to_string().contains("fanout"));
        assert!(serde_json::from_str::<SearchMode>(r#""fallback""#).is_err());
    }

    #[test]
    fn target_groups_match_agent_layout() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let targets = skill_targets(AgentChoice::All, InstallScope::Project).unwrap();
        assert_eq!(
            targets,
            [
                PathBuf::from(".claude/skills/kestrelsearch/SKILL.md"),
                PathBuf::from(".github/skills/kestrelsearch/SKILL.md"),
                PathBuf::from(".codex/skills/kestrelsearch/SKILL.md"),
            ]
        );
    }

    #[test]
    fn selections_reject_invalid_entries() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let paths = vec![PathBuf::from("one"), PathBuf::from("two")];
        assert_eq!(select_installations("2,2,nope,3", &paths), [&paths[1]]);
    }

    #[tokio::test]
    async fn gated_search_normalizes_queries_for_search_and_selection() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let Commands::Search(mut args) = Cli::try_parse_from([
            "kestrel",
            "search",
            "  rust  ",
            "--query",
            " ",
            "--query",
            "python",
            "--query",
            "rust",
            "--min-fetch-score",
            "0.1",
        ])
        .unwrap()
        .command
        else {
            panic!("expected search");
        };
        assert_eq!(args.queries(), ["rust", "python"]);
        let input: Vec<SearchResult> = serde_json::from_value(serde_json::json!([
            {"title":"python", "url":"https://example.com/first", "display_url":"", "snippet":"", "content":null, "query":"rust"},
            {"title":"python", "url":"https://example.com/second", "display_url":"", "snippet":"", "content":null, "query":"python"}
        ])).unwrap();
        let selected = select_fetch_candidates(input, &args, &args.queries(), &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(selected.results.len(), 1);
        assert_eq!(selected.results[0].query.as_deref(), Some("python"));
        args.min_fetch_score = None;
        assert_eq!(args.queries(), ["  rust  ", " ", "python", "rust"]);
    }

    #[tokio::test]
    async fn fetch_threshold_filters_before_cap_and_cache_without_refilling_failures() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        for endpoint in ["/irrelevant", "/beyond-cap"] {
            Mock::given(path(endpoint))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&server)
                .await;
        }
        Mock::given(path("/good"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("rust evidence"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/failed"))
            .respond_with(ResponseTemplate::new(404))
            .expect(2)
            .mount(&server)
            .await;
        let Commands::Search(mut args) = Cli::try_parse_from([
            "kestrel",
            "search",
            "rust",
            "--min-fetch-score",
            "0.01",
            "--fetch-candidates",
            "2",
            "--no-rank",
        ])
        .unwrap()
        .command
        else {
            panic!("expected search");
        };
        let directory = tempfile::tempdir().unwrap();
        args.cache_ttl = Some(60.0);
        args.cache_dir = Some(directory.path().join("cache"));
        let client = KestrelClient::new().unwrap();
        let candidate = |endpoint: &str, title: &str| SearchResult {
            title: title.into(),
            url: format!("{}{endpoint}", server.uri()),
            display_url: String::new(),
            snippet: String::new(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: Vec::new(),
        };
        for expected_cache_hits in [0, 1] {
            let input = vec![
                candidate("/irrelevant", "cooking"),
                candidate("/good", "rust"),
                candidate("/failed", "rust"),
                candidate("/beyond-cap", "rust"),
            ];
            let mut diagnostics = Vec::new();
            let mut selected =
                select_fetch_candidates(input, &args, &["rust".into()], &mut diagnostics)
                    .await
                    .unwrap();
            assert_eq!(selected.counts["after_fetch_score"], 3);
            assert_eq!(selected.counts["fetch_score_rejected"], 1);
            assert_eq!(selected.results.len(), 2);
            assert!(selected.results[0].url.ends_with("/good"));
            let report =
                attach_page_content(&client, &mut selected.results, &args, &mut diagnostics)
                    .await
                    .unwrap();
            assert_eq!(report.cache_hits, expected_cache_hits);
            assert!(selected.results[0].content.is_some());
            assert!(selected.results[1].content.is_none());
            assert!(selected.results.iter().all(|r| r.bm25_score.is_none()));
            assert!(
                String::from_utf8(diagnostics)
                    .unwrap()
                    .contains("excluded 1 candidate(s)")
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn fetch_threshold_small_pool_empty_selection_and_absent_flag() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        Mock::given(path("/good"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("rust evidence"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/bad"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let Commands::Search(mut args) =
            Cli::try_parse_from(["kestrel", "search", "rust", "--min-fetch-score", "0.01"])
                .unwrap()
                .command
        else {
            panic!("expected search");
        };
        let candidate = |endpoint: &str, title: &str| SearchResult {
            title: title.into(),
            url: format!("{}{endpoint}", server.uri()),
            display_url: String::new(),
            snippet: String::new(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: Vec::new(),
        };
        let input = vec![candidate("/bad", "cooking"), candidate("/good", "rust")];
        let mut diagnostics = Vec::new();
        let client = KestrelClient::new().unwrap();
        // Gate applies below the default cap, also when pre-rank is enabled.
        args.pre_rank = true;
        let mut selected =
            select_fetch_candidates(input.clone(), &args, &["rust".into()], &mut diagnostics)
                .await
                .unwrap();
        assert_eq!(selected.results.len(), 1);
        assert!(!selected.timings.contains_key("pre_rank"));
        attach_page_content(&client, &mut selected.results, &args, &mut diagnostics)
            .await
            .unwrap();
        args.min_fetch_score = Some(f64::MAX);
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("must-not-be-created");
        args.cache_ttl = Some(60.0);
        args.cache_dir = Some(cache_path.clone());
        let mut selected =
            select_fetch_candidates(input.clone(), &args, &["rust".into()], &mut diagnostics)
                .await
                .unwrap();
        assert!(selected.results.is_empty());
        let report = attach_page_content(&client, &mut selected.results, &args, &mut diagnostics)
            .await
            .unwrap();
        assert!(report.pages.is_empty());
        assert_eq!(report.cache_misses, 0);
        assert!(!cache_path.exists());
        args.min_fetch_score = None;
        let selected =
            select_fetch_candidates(input.clone(), &args, &["rust".into()], &mut diagnostics)
                .await
                .unwrap();
        assert_eq!(selected.results, input);
        assert!(selected.counts.is_empty());
        assert!(selected.timings.is_empty());
        server.verify().await;
    }

    #[tokio::test]
    async fn fetch_threshold_skill_recipe_and_pre_rank_preserve_selection_contract() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let Commands::Search(mut args) = Cli::try_parse_from([
            "kestrel",
            "search",
            "rust async",
            "--min-results",
            "15",
            "--fetch-candidates",
            "8",
            "--min-fetch-score",
            "0.1",
            "--no-rank",
        ])
        .unwrap()
        .command
        else {
            panic!("expected search");
        };
        let input: Vec<_> = (0..11)
            .map(|index| SearchResult {
                title: if index == 0 {
                    "cooking".into()
                } else {
                    "rust async".into()
                },
                url: format!("https://example.com/{index}"),
                display_url: String::new(),
                snippet: String::new(),
                content: None,
                bm25_score: None,
                engine: Some(Engine::Bing),
                query: Some("rust async".into()),
                engine_rank: Some(index + 1),
                sources: Vec::new(),
            })
            .collect();
        for pre_rank in [false, true] {
            args.pre_rank = pre_rank;
            let selected = select_fetch_candidates(
                input.clone(),
                &args,
                &["rust async".into()],
                &mut Vec::new(),
            )
            .await
            .unwrap();
            assert_eq!(selected.results, input[1..9]);
            assert_eq!(selected.counts["after_fetch_score"], 10);
            assert_eq!(selected.timings.contains_key("pre_rank"), pre_rank);
        }
        let mut diagnostics = Vec::new();
        let mut input = input;
        for hit in &mut input {
            hit.query = Some("!!!".into());
        }
        let selected = select_fetch_candidates(input, &args, &["!!!".into()], &mut diagnostics)
            .await
            .unwrap();
        assert_eq!(selected.counts["fetch_score_bypassed_queries"], 1);
        assert!(
            String::from_utf8(diagnostics)
                .unwrap()
                .contains("without lexical terms")
        );
    }

    #[tokio::test]
    async fn search_fetch_warns_for_successful_capped_pages_including_cache_misses() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        let short = "<p>This short response has readable content.</p>";
        for (path_name, body) in [
            ("/short", short.to_owned()),
            (
                "/capped",
                format!("<p>{}</p>", "Readable page content. ".repeat(50)),
            ),
            ("/empty", format!("<script>{}</script>", "x".repeat(500))),
        ] {
            Mock::given(path(path_name))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/html")
                        .set_body_bytes(body),
                )
                .mount(&server)
                .await;
        }
        let plain = "  Source: literal\n<p>&amp;</p>\n";
        let markdown = "# Heading\n\n- [link](https://example.org)\n```html\n<p>&amp;</p>\n```\n";
        Mock::given(path("/markdown"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(markdown, "text/markdown"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/plain"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(plain)
                    .insert_header("content-type", "text/plain"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = KestrelClient::new().unwrap();
        let Commands::Search(mut args) = Cli::try_parse_from([
            "kestrel",
            "search",
            "readable",
            "--max-response-bytes",
            "100",
        ])
        .unwrap()
        .command
        else {
            panic!("expected search");
        };
        let directory = tempfile::tempdir().unwrap();
        args.cache_ttl = Some(60.0);
        args.cache_dir = Some(directory.path().to_owned());
        let candidate = |path: &str| SearchResult {
            title: "Page".into(),
            url: format!("{}{path}", server.uri()),
            display_url: String::new(),
            snippet: "Snippet".into(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: Vec::new(),
        };
        for expected_hits in [0, 1] {
            let mut results: Vec<_> = ["/short", "/capped", "/empty", "/plain", "/markdown"]
                .into_iter()
                .map(candidate)
                .collect();
            let mut diagnostics = Vec::new();
            let report = attach_page_content(&client, &mut results, &args, &mut diagnostics)
                .await
                .unwrap();
            let diagnostics = String::from_utf8(diagnostics).unwrap();
            assert!(diagnostics.contains("1 fetched page(s) reached --max-response-bytes"));
            assert!(diagnostics.contains("search results may contain incomplete page content"));
            assert!(!diagnostics.contains("2 fetched page(s)"));
            assert_eq!(report.cache_hits, expected_hits * 3);
            assert_eq!(
                results[4].content.as_deref(),
                Some(format!("Source: {}/markdown\n\n{markdown}", server.uri()).as_str())
            );
            assert!(results[0].content.is_some());
            assert!(results[1].content.is_some());
            assert!(results[2].content.is_none());
            assert_eq!(
                results[3].content.as_deref(),
                Some(format!("Source: {}/plain\n\n{plain}", server.uri()).as_str())
            );
        }
        let mut results = [candidate("/short")];
        let mut diagnostics = Vec::new();
        attach_page_content(&client, &mut results, &args, &mut diagnostics)
            .await
            .unwrap();
        assert!(
            !String::from_utf8(diagnostics)
                .unwrap()
                .contains("reached --max-response-bytes")
        );
    }

    #[tokio::test]
    async fn quality_does_not_reject_search_bodies_or_change_ranking() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        use kestrelsearch::{
            ContentQualityState,
            ranking::{RankingPolicy, rank_with_policy},
        };
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        Mock::given(path("/shell"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("Your browser is not supported.", "text/plain"),
            )
            .mount(&server)
            .await;
        let mut results: Vec<SearchResult> = serde_json::from_value(serde_json::json!([{
            "title": "Browser", "url": format!("{}/shell", server.uri()),
            "display_url": "example.test", "snippet": "Browser help", "content": null
        }]))
        .unwrap();
        let Commands::Search(args) = Cli::try_parse_from(["kestrel", "search", "browser"])
            .unwrap()
            .command
        else {
            panic!("expected search");
        };
        attach_page_content(
            &KestrelClient::new().unwrap(),
            &mut results,
            &args,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            results[0].content_quality().state,
            ContentQualityState::BoilerplateOnly
        );
        for policy in [RankingPolicy::Body, RankingPolicy::Hybrid] {
            let ranked = rank_with_policy(results.clone(), &["browser".into()], policy);
            assert_eq!(ranked.len(), 1);
            assert_eq!(ranked[0].content, results[0].content);
            assert_eq!(ranked[0].title, "Browser");
            assert_eq!(ranked[0].snippet, "Browser help");
        }
    }

    #[test]
    fn page_fetch_defaults_match_library_and_preserve_character_limits() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let library = FetchOptions::default();
        assert_eq!(library.max_response_bytes, 1_000_000);
        assert_eq!(library.content_limit, 2_000);
        let Commands::Search(search) = Cli::try_parse_from(["kestrel", "search", "test"])
            .unwrap()
            .command
        else {
            panic!("expected search");
        };
        let Commands::Fetch(fetch) =
            Cli::try_parse_from(["kestrel", "fetch", "https://example.com"])
                .unwrap()
                .command
        else {
            panic!("expected fetch");
        };
        assert_eq!(search.max_response_bytes, library.max_response_bytes);
        assert_eq!(fetch.max_response_bytes, library.max_response_bytes);
        assert_eq!(search.content_limit, 2_000);
        assert_eq!(fetch.content_limit, 20_000);
    }

    #[test]
    fn generated_skill_documents_discovery_retry_contract() {
        let skill = generate_skill_md(&mut Cli::command());
        for term in [
            "32s",
            "discovery_attempt",
            "rate_limited_deadline",
            "nine total",
            "B >=15s",
            "no JSON error envelope",
        ] {
            assert!(skill.contains(term), "missing {term}");
        }
        let mut root = Cli::command();
        let help = root
            .find_subcommand_mut("search")
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(help.contains("Budgets >=15s use one attempt"));
    }

    #[test]
    fn generated_skill_documents_positive_body_bm25() {
        let skill = generate_skill_md(&mut Cli::command());
        for contract in [
            "`bm25` crate",
            "positive IDF",
            "k1=1.5",
            "computed in f32",
            "stable ties",
        ] {
            assert!(skill.contains(contract), "missing {contract}");
        }
    }

    #[test]
    fn generated_skill_documents_ranking_statistics() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("without tokenizing titles, snippets or bodies"));
        assert!(skill.contains("precompute query-term BM25 statistics"));
        assert!(skill.contains("scores, inclusive thresholds and stable ties are unchanged"));
    }

    #[test]
    fn generated_skill_documents_local_diagnostic_delivery() {
        let skill = generate_skill_md(&mut Cli::command());
        for contract in [
            "64\n  queued/preparing records",
            "16 MiB",
            "8 MiB per record",
            "one-second flush",
            "diagnostic_sink::Config",
            "diagnostic_sink::flush",
            "Explicit benchmark artifacts retain synchronous writes",
        ] {
            assert!(
                skill.contains(contract),
                "missing diagnostic contract: {contract}"
            );
        }
    }

    #[test]
    fn generated_skill_distinguishes_semantic_primitives_from_cli() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("Hybrid is lexical, not semantic search"));
        assert!(skill.contains("Final ordering (default: hybrid"));
        assert!(skill.contains("`--no-fetch` ranks metadata alone"));
        assert!(
            skill.contains("Hybrid scores stay internal and `bm25_score` is absent under hybrid")
        );
        assert!(skill.contains("have no production backend or CLI mode"));
    }

    #[test]
    fn generated_skill_documents_conditional_candidate_capture() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("only enabled artifact capture retains a full"));
        assert!(skill.contains("without retaining duplicate page bodies"));
        assert!(skill.contains("KESTRELSEARCH_BENCHMARK_RUN_ID"));
    }

    #[test]
    fn generated_skill_documents_cache_deadline_and_limits() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("cache reads, page requests, writes and maintenance"));
        assert!(skill.contains("four blocking storage jobs"));
        assert!(skill.contains("4,096 directory entries"));
        assert!(skill.contains("already-running blocking I/O may finish"));
    }

    #[test]
    fn generated_skill_documents_conservative_cache_identity() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("Cache keys preserve"));
        assert!(skill.contains("Legacy unversioned and page-text-v2 entries are misses"));
        assert!(skill.contains("deduplication remains unchanged"));
    }

    #[test]
    fn generated_skill_documents_result_minimum_precedence() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("--min-results"));
        assert!(skill.contains("Provider quorum is ignored"));
        assert!(skill.contains(
            "Omitting the minimum still means five, regardless of quorum or search budget"
        ));
        assert!(skill.contains("five\n  unique accepted candidates"));
        assert!(skill.contains("Query constraints apply before counting"));
        assert!(!skill.contains("selects quorum 1"));
    }

    #[test]
    fn generated_skill_documents_provider_worker_ownership() {
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("ten queued/running blocking workers per retained client"));
        assert!(skill.contains("Cancellation retains capacity until the worker exits"));
        assert!(skill.contains("Parser queueing is included in the search budget"));
    }

    #[test]
    fn generated_skill_examples_parse_with_current_cli() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let skill = generate_skill_md(&mut Cli::command());
        let mut count = 0;
        for block in skill.split("```bash\n").skip(1) {
            for line in block.split("```").next().unwrap().lines() {
                let words = shlex::split(line).expect("valid shell quoting in skill example");
                let Some(start) = words.iter().position(|word| word == "kestrel") else {
                    continue;
                };
                assert!(words[..start].iter().all(|word| word.contains('=')));
                let cli = Cli::try_parse_from(&words[start..])
                    .unwrap_or_else(|error| panic!("Invalid example: {line}\n{error}"));
                if let Commands::Search(args) = cli.command {
                    assert!(
                        !(args.no_fetch
                            && matches!(
                                args.ranking_policy,
                                Some(kestrelsearch::ranking::RankingPolicy::Body)
                            ))
                    );
                }
                count += 1;
            }
        }
        assert!(
            count >= 12,
            "expected search, fetch, and skill refresh examples"
        );
    }

    #[test]
    fn generated_skill_documents_parser_conflicts_and_cache_requirements() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let skill = generate_skill_md(&mut Cli::command());
        for (left, right) in [
            (vec!["--fetch"], vec!["--no-fetch"]),
            (vec!["--rank"], vec!["--no-rank"]),
            (vec!["--rank"], vec!["--no-fetch"]),
            (vec!["--rank"], vec!["--ranking-policy", "snippet"]),
            (vec!["--no-fetch"], vec!["--pre-rank"]),
            (vec!["--no-fetch"], vec!["--timeout", "10"]),
            (vec!["--search-budget", "3"], vec!["--no-search-budget"]),
            (vec!["--ranking-policy", "snippet"], vec!["--no-rank"]),
        ] {
            let mut command = Cli::command();
            command.build();
            let search = command.find_subcommand("search").unwrap();
            let syntax = |flag: &str| {
                search
                    .get_arguments()
                    .find(|arg| arg.get_long() == flag.strip_prefix("--"))
                    .unwrap()
                    .to_string()
            };
            let mut pair = [syntax(left[0]), syntax(right[0])];
            pair.sort();
            assert!(skill.contains(&format!("- `{}` and `{}`", pair[0], pair[1])));
            let error = Cli::try_parse_from(
                ["kestrel", "search", "test"]
                    .into_iter()
                    .chain(left)
                    .chain(right),
            )
            .unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
        for option in [["--cache-dir", "cache"], ["--cache-max-entries", "10"]] {
            let error =
                Cli::try_parse_from(["kestrel", "search", "test"].into_iter().chain(option))
                    .unwrap_err();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
            assert!(skill.contains(&format!("`{}`", option[0])));
            assert!(skill.contains("require `--cache-ttl`"));
        }
    }

    #[test]
    fn generated_skill_schema_matches_serialized_results() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let skill = generate_skill_md(&mut Cli::command());
        let mut result = SearchResult {
            title: "Example".into(),
            url: "https://example.com/".into(),
            display_url: "example.com".into(),
            snippet: "Example snippet".into(),
            content: None,
            bm25_score: None,
            engine: None,
            query: None,
            engine_rank: None,
            sources: vec![],
        };
        let minimal = serde_json::to_value(&result).unwrap();
        assert!(minimal["content"].is_null());
        result.bm25_score = Some(1.0);
        result.engine = Some(Engine::Bing);
        result.query = Some("example".into());
        result.engine_rank = Some(1);
        result.sources = serde_json::from_value(serde_json::json!([
            {"engine": "bing", "query": "example", "rank": 1}
        ]))
        .unwrap();
        let complete = serde_json::to_value(&result).unwrap();
        let started = Instant::now() - Duration::from_millis(1250);
        for results in [std::slice::from_ref(&result), &[]] {
            let json: serde_json::Value =
                serde_json::from_str(&search_json(results, started, None).unwrap()).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 2);
            assert_eq!(json["results"], serde_json::to_value(results).unwrap());
            let seconds = json["elapsed_seconds"].as_f64().unwrap();
            assert!(seconds.is_finite() && seconds >= 1.25);
            assert!(seconds <= started.elapsed().as_secs_f64());
        }

        let schema = skill
            .split_once("## Search JSON output schema\n")
            .unwrap()
            .1
            .split_once("## Notes\n")
            .unwrap()
            .0;
        let documented_fields: Vec<_> = schema
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .map(|line| line.split('`').next().unwrap())
            .collect();
        assert_eq!(documented_fields.len(), complete.as_object().unwrap().len());
        for field in complete.as_object().unwrap().keys() {
            let row = schema
                .lines()
                .find(|line| line.starts_with(&format!("| `{field}` |")))
                .unwrap_or_else(|| panic!("Undocumented JSON field: {field}"));
            assert_eq!(
                row.contains(", optional |"),
                minimal.get(field).is_none(),
                "{field}"
            );
        }
        assert_eq!(
            complete["sources"][0],
            serde_json::json!({
                "engine": "bing", "query": "example", "rank": 1
            })
        );
        assert!(skill.contains(&format!("cargo install {}", env!("CARGO_PKG_NAME"))));
    }

    #[test]
    fn structured_search_json_default_and_opt_out_preserve_results() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for opt_out in [false, true] {
            let mut args = vec!["kestrel", "search", "q", "--no-fetch", "--output", "json"];
            if opt_out {
                args.push("--no-diagnostics");
            }
            let Cli {
                command: Commands::Search(args),
            } = Cli::try_parse_from(args).unwrap()
            else {
                panic!("search");
            };
            assert_eq!(args.no_diagnostics, opt_out);
            let report = kestrelsearch::SearchReport {
                results: vec![],
                providers: vec![],
                cancelled: 0,
            };
            let diagnostic = (!args.no_diagnostics).then(|| {
                diagnostics::SearchDiagnostics::new(&report, &["q".into()], 5).finish(
                    &[],
                    &[],
                    None,
                    true,
                    &BTreeMap::new(),
                    100,
                )
            });
            let value: serde_json::Value = serde_json::from_str(
                &search_json(&[], Instant::now(), diagnostic.as_ref()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                value.as_object().unwrap().len(),
                if opt_out { 2 } else { 3 }
            );
            assert_eq!(value["results"], serde_json::json!([]));
            assert!(value["elapsed_seconds"].is_number());
            if !opt_out {
                assert_eq!(value["diagnostics"]["schema_version"], 1);
            }
        }
    }

    #[test]
    fn generated_skill_reflects_cli() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let skill = generate_skill_md(&mut Cli::command());
        assert!(skill.contains("name: kestrelsearch"));
        assert!(skill.contains("--time-filter"));
        assert!(skill.contains("--max-response-bytes"));
        assert!(skill.contains("Codex"));
        assert!(skill.contains("## `fetch` subcommand"));
        assert!(skill.contains("kestrel fetch"));
        assert!(skill.contains("Do not use `search \"site:<full-url-to-page>\"`"));
        assert!(skill.contains("## Fetch output"));
        assert!(skill.contains("20000"));
        assert!(skill.contains("retained prefix"));
        assert!(skill.contains("Search reports the number of successfully extracted pages"));
        assert!(skill.contains("exit status zero"));
        assert!(skill.contains("not cached"));
        assert!(skill.contains("--max-response-bytes 65536"));
        assert!(skill.contains("defaults to 1,000,000 decoded body bytes"));
        assert_eq!(skill.matches("[default: 1000000]").count(), 2);
    }
}

#[cfg(test)]
#[path = "cli/recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[test]
fn skill_documents_incremental_page_recovery() {
    let skill = kestrelsearch::skill::generate_skill_md(&mut Cli::command());
    for phrase in [
        "Eligible pages commit incrementally",
        "16 entries and 16 MiB",
        "page-text-v2",
        "response-byte allowance",
        "page-only",
    ] {
        assert!(skill.contains(phrase), "missing {phrase}");
    }
}

#[cfg(test)]
#[test]
fn recovery_flags_and_generated_skill_match_replay_contract() {
    for args in [
        vec!["--no-fetch", "--recovery-ttl", "60"],
        vec![
            "--recovery-ttl",
            "60",
            "--recovery-dir",
            "progress",
            "--recovery-max-entries",
            "10",
        ],
    ] {
        assert!(Cli::try_parse_from([vec!["kestrel", "search", "fixture"], args].concat()).is_ok());
    }
    for args in [
        vec!["--recovery-dir", "progress"],
        vec!["--recovery-max-entries", "10"],
        vec!["--recovery-ttl", "0"],
        vec!["--recovery-ttl", "NaN"],
    ] {
        assert!(
            Cli::try_parse_from([vec!["kestrel", "search", "fixture"], args].concat()).is_err()
        );
    }
    let skill = generate_skill_md(&mut Cli::command());
    for text in [
        "--recovery-ttl",
        "--recovery-dir",
        "--recovery-max-entries",
        "Recovering interrupted searches",
        "64 MiB",
        "invalid tombstone",
    ] {
        assert!(skill.contains(text), "missing {text}");
    }
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => Some(signal),
                Err(error) => {
                    eprintln!("[kestrel] SIGTERM handler unavailable: {error}");
                    None
                }
            };
        let result = tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            () = async { match &mut terminate { Some(signal) => { let _ = signal.recv().await; }, None => std::future::pending().await } } => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("[kestrel] Interrupt handler unavailable: {error}");
            std::future::pending::<()>().await;
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("[kestrel] Interrupt handler unavailable: {error}");
        std::future::pending::<()>().await;
    }
}
