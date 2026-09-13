# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
