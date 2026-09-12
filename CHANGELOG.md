# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
