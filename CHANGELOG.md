# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [6.0.4](https://github.com/rafaelpierre/kestrel-rs/compare/v6.0.3...v6.0.4) - 2026-09-14

### Fixed

- synchronize diagnostic deadline tests with request phases

### Other

- Merge pull request #177 from rafaelpierre/codex/174-markdown-fetch

## [6.0.3](https://github.com/rafaelpierre/kestrel-rs/compare/v6.0.2...v6.0.3) - 2026-09-14

### Fixed

- make recovery queue regression independent of disk timing

### Other

- configure Dependabot security updates and MSRV checks

## [6.0.2](https://github.com/rafaelpierre/kestrel-rs/compare/v6.0.1...v6.0.2) - 2026-09-13

### Fixed

- reconcile cache deadlines and search recovery with main

### Other

- Merge pull request #155 from rafaelpierre/release-plz-2026-09-13T16-48-23Z
- Merge branch 'main' into codex/46-streaming-evidence

## [6.0.1](https://github.com/rafaelpierre/kestrel-rs/compare/v6.0.0...v6.0.1) - 2026-09-13

### Fixed

- preserve HTTP resource distinctions in persistent page cache
- bound page parsers across reusable client calls

### Other

- Merge pull request #163 from rafaelpierre/codex/75-fixed-pool-study
- Merge pull request #162 from rafaelpierre/codex/148-evidence-gate
- Merge pull request #161 from rafaelpierre/codex/121-recovery-contract
- Merge pull request #159 from rafaelpierre/codex/117-provider-parser-workers
- Merge pull request #158 from rafaelpierre/codex/116-parse-provider-once
- investigate Yahoo empty-500 redirects and retry costs
- Merge pull request #152 from rafaelpierre/release-plz-2026-09-13T16-22-00Z
- preserve exact error wording in keyword queries
- require keyword FTS queries in generated skill and evidence gate

## [6.0.0](https://github.com/rafaelpierre/kestrel-rs/compare/v5.0.0...v6.0.0) - 2026-09-13

### Added

- [**breaking**] add default structured CLI diagnostics

### Other

- Merge pull request #149 from rafaelpierre/release-plz-2026-09-13T15-39-09Z
- isolate structured diagnostics fixtures from proxy settings

## [5.0.0](https://github.com/rafaelpierre/kestrel-rs/compare/v4.1.1...v5.0.0) - 2026-09-13

### Added

- add OpenTelemetry and Honeycomb test tracing

### Fixed

- preserve document order and code in HTML extraction
- [**breaking**] remove portable query filtering
- pin portable syntax in streaming validation
- [**breaking**] restore native query passthrough by default

### Other

- Merge pull request #145 from rafaelpierre/release-plz-2026-09-13T14-42-39Z
- merge latest main before query filter removal
- merge main into native query default fix

## [4.1.1](https://github.com/rafaelpierre/kestrel-rs/compare/v4.1.0...v4.1.1) - 2026-09-13

### Fixed

- recover interrupted benchmark records and isolate fixture proxies
- replace obsolete quality benchmark conditions

### Other

- Merge pull request #140 from rafaelpierre/codex/75-benchmark-ablation
- add current-contract streaming fanout validation ([#131](https://github.com/rafaelpierre/kestrel-rs/pull/131))
- Merge pull request #129 from rafaelpierre/codex/32-current-fidelity

## [4.1.0](https://github.com/rafaelpierre/kestrel-rs/compare/v4.0.1...v4.1.0) - 2026-09-13

### Added

- add opt-in metadata threshold before page fetching ([#130](https://github.com/rafaelpierre/kestrel-rs/pull/130))
- add opt-in metadata threshold before page fetching
- assess extracted content and recover shell-only roots

### Fixed

- normalize queries before fetch-score filtering

### Other

- Merge pull request #94 from rafaelpierre/codex/25-atomic-config
- Merge pull request #95 from rafaelpierre/codex/77-content-quality
- Merge pull request #88 from rafaelpierre/codex/82-adaptive-skill
- teach bounded discovery and selective reading in generated skill

## [4.0.1](https://github.com/rafaelpierre/kestrel-rs/compare/v4.0.0...v4.0.1) - 2026-09-13

### Fixed

- reject overflowing search and fetch numeric options
- preserve plain-text responses during page extraction

### Other

- Merge pull request #87 from rafaelpierre/codex/21-plain-text-extraction

## [4.0.0](https://github.com/rafaelpierre/kestrel-rs/compare/v3.0.0...v4.0.0) - 2026-09-13

### Fixed

- preserve content with incidental chrome substrings
- [**breaking**] clarify search controls and reject conflicting arguments

### Other

- Merge pull request #49 from rafaelpierre/release-plz-2026-09-12T15-11-55Z

## [3.0.0](https://github.com/rafaelpierre/kestrel-rs/compare/v2.0.0...v3.0.0) - 2026-09-12

### Added

- [**breaking**] include elapsed seconds in search and fetch JSON
- search all nine supported engines by default

### Fixed

- return partial pages with a 1 MB fetch default
- bound fanout search latency and raise concurrency defaults
- count isolated Bing attempts on errors and deadlines
- default budgeted CLI searches to fanout quorum one

### Other

- Merge pull request #58 from rafaelpierre/codex/53-partial-fetch
- Merge pull request #63 from rafaelpierre/codex/61-elapsed-seconds
- Merge pull request #60 from rafaelpierre/codex/59-github-api-signing
- document verified GitHub API commit signing
- Merge pull request #56 from rafaelpierre/codex/issue-55-agents-guidelines
- add repository agent development guidelines
- Merge pull request #45 from rafaelpierre/codex/issue-32-bing-fidelity
- Preserve budgeted quorum defaults and resolve fanout-only integration
- Prepare verified integration of current main

### Changed

- Search now defaults to portable title/snippet query constraints across all nine providers. Supports phrases, Boolean expressions, exclusions and hostname restrictions before quorum. Use `--query-syntax native` or `SearchOptions.query_syntax = QuerySyntax::Native` for previous passthrough behavior. Explicit SearchOptions literals require the new field.

## [2.0.0](https://github.com/rafaelpierre/kestrel-rs/compare/v1.1.1...v2.0.0) - 2026-09-12

### Fixed

- preserve structured provider diagnostics through cancellation
- classify Mojeek CAPTCHA responses as challenges
- bound decoded provider responses and preserve status retries
- bound page bodies waiting for parsing
- authenticate GitHub release publishing

### Other

- Remove fallback search mode and keep fanout only
- Align generated release workflow with cargo-dist configuration
- Merge remote-tracking branch 'origin/main' into codex/http2-optimization
- Merge main and preserve browser profiles with HTTP/2 tuning
- randomized headers/6
- reinstall
- install
- fetch
- record distribution publishing checkpoint
- update installation instructions
- enable automated crates.io publishing

## [1.1.1](https://github.com/rafaelpierre/kestrel-rs/releases/tag/v1.1.1) - 2026-08-28

### Fixed

- publish the Cargo package under the `kestrel-rs` crate name

## [1.1.0](https://github.com/rafaelpierre/kestrel-rs/releases/tag/v1.1.0) - 2026-08-28

### Added

- automate Apple Silicon releases

### Fixed

- use supported release dispatch

### Other

- Initial project import
- Initial commit
