use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum};
use kestrelsearch::benchmarking::{ArtifactDiagnostics, write_artifact};
use kestrelsearch::config::{
    config_path, get_installations, record_installation, remove_installation,
};
use kestrelsearch::fetcher::DEFAULT_MAX_RESPONSE_BYTES;
use kestrelsearch::skill::generate_skill_md;
use kestrelsearch::{
    Engine, FetchOptions, FetchReport, KestrelClient, PageCache, SearchMode, SearchOptions,
    SearchResult, TimeFilter, pre_rank_candidates, rank_results_by_query,
};

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
    /// Fetch a URL directly and extract its page text without searching.
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

    /// Query syntax: portable checks titles/snippets; native passes provider syntax through.
    #[arg(long, value_enum, default_value = "portable")]
    query_syntax: kestrelsearch::QuerySyntax,

    /// Additional query to run. Repeat for multiple queries.
    #[arg(short = 'q', long = "query", value_name = "QUERY")]
    additional_queries: Vec<String>,

    /// Search engine. Repeat to select providers to search concurrently.
    #[arg(short = 'e', long = "engine", default_values = ["duckduckgo", "bing", "yahoo", "dogpile", "ecosia", "swisscows", "yep", "qwant", "mojeek"], action = ArgAction::Append)]
    engines: Vec<Engine>,

    /// Compatibility option: fanout is the only supported search mode.
    #[arg(long)]
    mode: Option<SearchMode>,

    /// Maximum concurrent search-engine requests.
    #[arg(long, default_value_t = 10, value_parser = positive_usize)]
    search_concurrency: usize,

    /// Stop fanout after N nonempty responses (default: 1 with implicit budgeted fanout).
    #[arg(long, value_parser = positive_usize, value_name = "N")]
    provider_quorum: Option<usize>,

    /// Total search seconds, including queueing and retries (default: 5).
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    search_budget: Option<f64>,

    /// Disable the total search deadline; per-request timeouts still apply.
    #[arg(long, conflicts_with = "search_budget")]
    no_search_budget: bool,

    /// Number of top results to return.
    #[arg(short = 'k', long, default_value_t = 5, value_parser = positive_usize, value_name = "N")]
    top_k: usize,

    /// Maximum candidates to fetch before ranking (default: 3 x top-k).
    #[arg(long, value_parser = positive_usize, value_name = "N")]
    fetch_candidates: Option<usize>,

    /// Pre-rank titles/snippets before selecting pages to fetch.
    #[arg(long)]
    pre_rank: bool,

    /// Explicitly enable page fetching (enabled by default).
    #[arg(long, conflicts_with = "no_fetch")]
    fetch: bool,

    /// Do not fetch or parse result pages.
    #[arg(long, conflicts_with = "fetch")]
    no_fetch: bool,

    /// Explicitly enable BM25 ranking (enabled by default and requires fetching).
    #[arg(long, conflicts_with = "no_rank")]
    rank: bool,

    /// Keep provider ordering instead of applying BM25 ranking.
    #[arg(long, conflicts_with = "rank")]
    no_rank: bool,

    /// Experimental final ordering (body requires page fetching).
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
    #[arg(long, default_value_t = 10.0, value_parser = positive_f64, value_name = "SECS")]
    timeout: f64,

    /// Total seconds allowed for all candidate page fetches; completed pages are retained.
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    fetch_budget: Option<f64>,

    /// Cache extracted page text for this many seconds (disabled by default).
    #[arg(long, value_parser = positive_f64, value_name = "SECS")]
    cache_ttl: Option<f64>,

    /// Directory for extracted-page cache entries.
    #[arg(long, value_name = "PATH", requires = "cache_ttl")]
    cache_dir: Option<PathBuf>,

    /// Maximum extracted-page cache entries.
    #[arg(long, value_parser = positive_usize, value_name = "N", requires = "cache_ttl")]
    cache_max_entries: Option<usize>,

    /// Maximum concurrent HTTP requests when fetching pages.
    #[arg(long, default_value_t = 10, value_parser = positive_usize, value_name = "N")]
    concurrency: usize,

    /// Maximum concurrent HTML parsing jobs.
    #[arg(long, default_value_t = 10, value_parser = positive_usize, value_name = "N")]
    parse_concurrency: usize,

    /// Output format. Use json for agent/programmatic consumption.
    #[arg(long, default_value = "text")]
    output: Output,
}

impl SearchArgs {
    fn effective_search_budget(&self) -> Option<Duration> {
        if self.no_search_budget {
            None
        } else {
            Some(Duration::from_secs_f64(self.search_budget.unwrap_or(5.0)))
        }
    }

    fn search_options(&self) -> SearchOptions {
        let budgeted_default = self.mode.is_none() && self.search_budget.is_some();
        SearchOptions {
            query_syntax: self.query_syntax,
            engines: self.engines.clone(),
            mode: self.mode.unwrap_or_default(),
            region: self.region.clone(),
            time_filter: self.time_filter,
            max_concurrency: self.search_concurrency,
            provider_quorum: self.provider_quorum.or(budgeted_default.then_some(1)),
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

    /// HTTP timeout in seconds.
    #[arg(long, default_value_t = 10.0, value_parser = positive_f64, value_name = "SECS")]
    timeout: f64,

    /// Output format. JSON returns an object with url and content fields.
    #[arg(long, default_value = "text")]
    output: Output,
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
    match Cli::parse().command {
        Commands::Install(arguments) => match crate::install::run(arguments) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("[kestrel] {error}");
                ExitCode::FAILURE
            }
        },
        Commands::Search(arguments) => run_search(*arguments).await,
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
    let Some(content) = report.contents.into_iter().next().flatten() else {
        use kestrelsearch::FetchOutcome;
        let reason = match report.pages.first().map(|page| page.outcome) {
            Some(FetchOutcome::UnsupportedContentType) => {
                "unsupported content type (expected HTML or text)"
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
    match arguments.output {
        Output::Text => println!("{content}"),
        Output::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "url": arguments.url,
                "content": content,
            }))
            .expect("strings serialize to JSON")
        ),
    }
    print_completion("Fetch", command_started);
    ExitCode::SUCCESS
}

async fn run_search(arguments: SearchArgs) -> ExitCode {
    let command_started = Instant::now();
    if arguments.no_fetch
        && matches!(
            arguments.ranking_policy,
            Some(kestrelsearch::ranking::RankingPolicy::Body)
        )
    {
        eprintln!("[kestrel] --ranking-policy body requires fetching");
        return ExitCode::FAILURE;
    }
    let mut queries = vec![arguments.query.clone()];
    queries.extend(arguments.additional_queries.clone());
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
    let client = match KestrelClient::new() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[kestrel] Failed to initialize HTTP clients: {error}");
            return ExitCode::FAILURE;
        }
    };
    timings.insert("initialize".into(), elapsed_millis(initialize_started));
    let started = Instant::now();
    let search_report = match client.search_many_detailed(&queries, &options).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[kestrel] Search failed: {error}");
            return ExitCode::FAILURE;
        }
    };
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
        eprintln!("[kestrel] Search stopped; cancelled {provider_cancellations} straggler(s).");
    }

    if results.is_empty() {
        write_benchmark_artifact(
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
        eprintln!("[kestrel] No results found.");
        println!(
            "{}",
            if arguments.output == Output::Json {
                "[]"
            } else {
                "No results found."
            }
        );
        print_completion("Search", command_started);
        return ExitCode::SUCCESS;
    }
    eprintln!("[kestrel] Got {} results.", results.len());

    let should_fetch = !arguments.no_fetch;
    let should_rank = !arguments.no_rank;
    let mut fetch_diagnostics = None;
    if should_fetch {
        let candidate_limit = arguments.fetch_candidates.unwrap_or(arguments.top_k * 3);
        if arguments.pre_rank && results.len() > candidate_limit {
            eprintln!("[kestrel] Pre-ranking candidates from titles and snippets...");
            let pre_rank_started = Instant::now();
            results = pre_rank_candidates(results, &queries);
            timings.insert("pre_rank".into(), elapsed_millis(pre_rank_started));
        }
        if results.len() > candidate_limit {
            eprintln!(
                "[kestrel] Fetching the first {candidate_limit} candidates before ranking (from {} search results).",
                results.len()
            );
            results.truncate(candidate_limit);
        }
        let fetch_started = Instant::now();
        fetch_diagnostics =
            match attach_page_content(&client, &mut results, &arguments, &mut io::stderr()).await {
                Ok(report) => Some(report),
                Err(error) => {
                    eprintln!("[kestrel] Fetch failed: {error}");
                    return ExitCode::FAILURE;
                }
            };
        timings.insert("fetch".into(), elapsed_millis(fetch_started));
    }

    candidate_counts.insert("after_candidate_selection".into(), results.len());
    candidate_counts.insert(
        "with_content".into(),
        results.iter().filter(|r| r.content.is_some()).count(),
    );
    let candidates = results.clone();
    if let Some(policy) = arguments.ranking_policy {
        let rank_started = Instant::now();
        results = kestrelsearch::ranking::rank_with_policy(results, &queries, policy);
        timings.insert("rank".into(), elapsed_millis(rank_started));
    } else if should_fetch && should_rank {
        eprintln!("[kestrel] Ranking with BM25...");
        let rank_started = Instant::now();
        results = rank_results_by_query(results, &queries);
        timings.insert("rank".into(), elapsed_millis(rank_started));
    }

    candidate_counts.insert("after_ranking".into(), results.len());
    results.truncate(arguments.top_k);
    candidate_counts.insert("returned".into(), results.len());
    write_benchmark_artifact(
        &query_label,
        &results,
        &timings,
        &queries,
        &options,
        ArtifactDiagnostics {
            providers: &provider_diagnostics,
            provider_cancellations,
            fetch: fetch_diagnostics.as_ref(),
            candidates: &candidates,
            candidate_counts: Some(&candidate_counts),
        },
    );
    eprintln!("[kestrel] Returning top {} results.", results.len());
    match arguments.output {
        Output::Json => match serde_json::to_string_pretty(&results) {
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
}

fn print_completion(command: &str, started: Instant) {
    eprintln!(
        "[kestrel] {command} completed in {:.3} seconds.",
        started.elapsed().as_secs_f64()
    );
}

async fn attach_page_content(
    client: &KestrelClient,
    results: &mut [SearchResult],
    arguments: &SearchArgs,
    diagnostics: &mut impl Write,
) -> Result<FetchReport, kestrelsearch::search::KestrelError> {
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
    let budget = arguments.fetch_budget.map(Duration::from_secs_f64);
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
    query: &str,
    results: &[SearchResult],
    timings: &BTreeMap<String, u64>,
    queries: &[String],
    options: &SearchOptions,
    diagnostics: ArtifactDiagnostics<'_>,
) {
    if let Err(error) = write_artifact(
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

fn positive_f64(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|error| error.to_string())
        .and_then(|value| {
            (value > 0.0 && value.is_finite())
                .then_some(value)
                .ok_or_else(|| "must be greater than zero".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_syntax_preserves_shell_argument_and_additional_queries() {
        for (flags, syntax) in [
            (vec![], kestrelsearch::QuerySyntax::Portable),
            (
                vec!["--query-syntax", "native"],
                kestrelsearch::QuerySyntax::Native,
            ),
        ] {
            let cli = Cli::try_parse_from(
                [
                    "kestrel",
                    "search",
                    r#""machine learning""#,
                    "--query",
                    "C++ AND Rust",
                ]
                .into_iter()
                .chain(flags),
            )
            .unwrap();
            let Commands::Search(args) = cli.command else {
                panic!("search expected")
            };
            assert_eq!(args.query, r#""machine learning""#);
            assert_eq!(args.additional_queries, ["C++ AND Rust"]);
            assert_eq!(args.search_options().query_syntax, syntax);
        }
    }

    #[test]
    fn search_budget_defaults_and_overrides() {
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
        for (flags, mode, quorum) in [
            (vec![], SearchMode::Fanout, None),
            (vec!["--search-budget", "3"], SearchMode::Fanout, Some(1)),
            (vec!["--search-budget", "0.5"], SearchMode::Fanout, Some(1)),
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
                Some(1),
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
        let error =
            Cli::try_parse_from(["kestrel", "search", "test", "--mode", "fallback"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert!(error.to_string().contains("fanout"));
        assert!(serde_json::from_str::<SearchMode>(r#""fallback""#).is_err());
    }

    #[test]
    fn target_groups_match_agent_layout() {
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
        let paths = vec![PathBuf::from("one"), PathBuf::from("two")];
        assert_eq!(select_installations("2,2,nope,3", &paths), [&paths[1]]);
    }

    #[tokio::test]
    async fn search_fetch_warns_for_successful_capped_pages_including_cache_misses() {
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
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
        }
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
            let mut results: Vec<_> = ["/short", "/capped", "/empty"]
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
            assert_eq!(report.cache_hits, expected_hits);
            assert!(results[0].content.is_some());
            assert!(results[1].content.is_some());
            assert!(results[2].content.is_none());
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

    #[test]
    fn page_fetch_defaults_match_library_and_preserve_character_limits() {
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
    fn generated_skill_examples_parse_with_current_cli() {
        let skill = generate_skill_md(&mut Cli::command());
        let mut count = 0;
        for block in skill.split("```bash\n").skip(1) {
            for line in block.split("```").next().unwrap().lines() {
                if !line.starts_with("kestrel ") {
                    continue;
                }
                let args = shlex::split(line).expect("valid shell quoting in skill example");
                let cli = Cli::try_parse_from(args)
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
        let skill = generate_skill_md(&mut Cli::command());
        for (left, right) in [
            (vec!["--fetch"], vec!["--no-fetch"]),
            (vec!["--rank"], vec!["--no-rank"]),
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
        let documented_fields: Vec<_> = skill
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .map(|line| line.split('`').next().unwrap())
            .collect();
        assert_eq!(documented_fields.len(), complete.as_object().unwrap().len());
        for field in complete.as_object().unwrap().keys() {
            let row = skill
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
    fn generated_skill_reflects_cli() {
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
