# Repository Guidelines

## Project Focus

- OxiDNS Next is a high-performance, plugin-driven DNS server written in Rust.
- The current project already includes UDP/TCP/DoT/DoQ/DoH server and upstream support, sequence-based policy orchestration, TTL-aware cache with negative caching, fallback chains, local and synthetic answers, query/response rewriting, ECS handling, dual-stack selection, provider-backed domain/IP rule sets, management APIs, health endpoints, metrics, and system integrations such as `ipset`, `nftset`, and MikroTik route sync.
- Prefer designs that preserve the core request path: `server -> DnsContext -> matcher/executor/provider pipeline -> upstream or side effects -> response`.

## Project Structure & Module Organization

- `src/main.rs` parses top-level CLI options, dispatches foreground startup or service mode, and keeps binary-only entry concerns thin.
- `src/lib.rs` exposes the library surface used by tests and embedding scenarios, including `api`, `app`, `build_info`, `cli`, `config`, `core`, `infra`, `plugin`, and the re-exported `proto` crate.
- `src/build_info.rs` reports compiled bundles, enabled features, and runtime plugin capabilities. It lives at the crate root because it depends on both infrastructure constants and the plugin catalog; do not move plugin-aware capability reporting into `infra`.
- `src/cli/` contains command definitions, parsing, command dispatch, CLI output, and option-to-runtime adapter code.
- `src/app/` contains foreground startup orchestration for wiring config, runtime, API, plugins, and graceful shutdown/reload flows.
- `src/api/` contains the management/control and health HTTP endpoints plus API route macros under `src/api/macros.rs`.
- `src/core/` is the DNS execution core and should stay focused on `DnsContext`, request lifecycle state, and reusable rule matching primitives.
- `src/infra/` contains subsystem-neutral infrastructure shared by CLI, API, app, and plugins: errors, clocks, environment helpers, line-oriented I/O, service management, task orchestration, TTL cache primitives, observability/logging/metrics, upgrade support, and networking.
- Keep the dependency direction one-way: `plugin` may use `infra`, but `infra` must not depend on plugin traits, registries, or plugin-specific models. Shared code belongs in `infra` only when its API and semantics are useful outside the plugin system.
- `src/config/` defines the YAML schema and validation for runtime configuration.
- `src/infra/network/` contains listeners, protocol transports, TLS setup, upstream resolution, bootstrap logic, pooling, and networking helpers.
- `src/infra/io/` contains reusable file and stream helpers, including line-oriented rule loading shared by providers.
- `src/infra/upgrade/` separates release discovery, download, archive handling, progress reporting, and binary/WebUI installation while exposing upgrade orchestration through `mod.rs`.
- `src/plugin/` is the main extension surface and is split into server, executor, matcher, and provider categories.
- `src/plugin/server/` handles inbound DNS protocols, including UDP, TCP, QUIC, and HTTP-based DNS. Category-local connection lifecycle, request handling, and metrics stay in this package; dedicated HTTP/2 and HTTP/3 support lives under `src/plugin/server/http/`.
- `src/plugin/executor/` contains request processors such as `sequence`, `forward`, `cache`, `fallback`, `hosts`, `arbitrary`, `redirect`, `ecs_handler`, `ttl`, `dual_selector`, observability plugins, and system-integration plugins.
- `src/plugin/matcher/` contains rule matchers for qname/qtype/qclass, client IP, response IP, CNAME, response presence, RCODE, marks, env, random rollout, rate limits, and related predicates. Shared matcher parsing, source classification, and provider binding stay under `src/plugin/matcher/rules/`.
- `src/plugin/provider/` contains reusable domain/IP datasets consumed by matchers and executors. Provider API wiring and V2Ray model/parser/selector helpers remain provider-local because they encode provider semantics.
- Service-management implementation lives in `src/infra/service.rs`; `src/cli/service.rs` only adapts CLI service options.
- `crates/macros/` provides proc-macros used by the plugin registration system (`register_plugin_factory!` and related derives).
- `crates/ripset/` is a pure-Rust Linux netlink implementation for ipset/nftset operations, used by the ipset and nftset executor plugins.
- `crates/proto/` contains OxiDNS Next's DNS message model and wire codec types (header, name, question, record, and rdata), re-exported by `src/lib.rs` as `proto`.
- `crates/zoneparser/` is a standalone zone-file parser used for loading hosts and local zone data.
- `tests/plugin_integration.rs` covers config parsing, plugin registry wiring, sequence quick-setup, and live server integration.
- `tests/message_hickory_compat.rs` validates message codec compatibility behavior against Hickory.
- `config.yaml` is the canonical runnable default configuration for the current plugin composition.
- `README.md` and `README_EN.md` describe the architecture and capability set; keep them aligned with behavior changes.
- Detailed internal architecture and dependency-boundary guidance lives in `ai/architecture.md`.
- WebUI-specific guidance lives in `ai/webui.md`; follow it for changes under `webui/`.

## Build, Test, and Development Commands

**Toolchain note:** `rustfmt.toml` uses `unstable_features = true`, so formatting and the pre-commit hook both require the nightly toolchain (`cargo +nightly fmt`). Install it with `rustup toolchain install nightly` if needed.

**Git hooks:** Run `just install-hooks` once per clone to activate the pre-commit hook (`cargo +nightly fmt --check` + `cargo +nightly clippy -- -D warnings`).

**Preferred quality gates (via `just`):**
- `just check` — full gate: fmt check + clippy (`-D warnings`) + tests. Run this before opening a PR.
- `just fix` — auto-applies fmt and Clippy fixes; use during active development.
- `just lint` — fmt check + clippy only, no tests; faster iteration cycle.

**Individual commands:**
- `cargo check` — fastest sanity check during iteration.
- `cargo build --release` — builds the optimized binary.
- `cargo run -- start -c config.yaml` — runs OxiDNS Next with the default config.
- `cargo run --release -- start -c config.yaml` — preferred for performance-sensitive validation.
- `cargo run -- start -c config.yaml -l debug` — overrides the log level for local debugging.
- `cargo test` — runs all unit and integration tests.
- `cargo test --test plugin_integration` — runs the end-to-end plugin/config integration suite.
- `cargo test <filter>` — runs tests whose names match the filter string (e.g., `cargo test cache` runs all cache-related tests).
- `cargo test --test plugin_integration <filter>` — runs a specific integration test by name.
- `cargo +nightly fmt` — formats code; nightly is required due to unstable rustfmt features.
- `cargo +nightly clippy --all-targets --all-features -- -D warnings` — lints with warnings as errors; required to match CI and the pre-commit hook.

## Coding Style & Naming Conventions

- Rust 2024 edition; format with `cargo +nightly fmt`.
- Use `snake_case` for functions and fields, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Keep modules cohesive and place helpers close to the feature they serve.
- Comments should be written in English.
- For plugin registration patterns, implementation guidelines, and platform-specific guarding rules, see [ai/plugin-dev.md](ai/plugin-dev.md).

## Performance & Architecture Principles

- Treat the request hot path as a first-class design constraint. Avoid unnecessary allocation, cloning, parsing, locking, or blocking I/O in per-request code.
- Prefer work that can be done once at startup or plugin initialization over work repeated for every query.
- Reuse connections and transport state through the existing upstream pool design instead of creating one-off connections on the fast path.
- Respect DNS semantics when touching cache, fallback, rewrite, or synthetic-response code, especially TTL and negative-cache behavior.
- Performance-sensitive changes must follow the hot-path and resource-safety review rules in `ai/performance.md`.
- For plugin-specific hot-path rules and composability principles, see [ai/plugin-dev.md](ai/plugin-dev.md).

## Testing Guidelines

- Use Rust's built-in test framework and keep focused unit tests close to logic-heavy modules.
- Prefer ephemeral ports, bounded timeouts, and deterministic inputs for network-facing tests.
- Run at least `cargo test` for behavior changes.
- Use `ai/testing-strategy.md` to select focused, bundle, feature-matrix, platform, WebUI, and docs validation.
- For plugin-specific testing rules (integration test placement, feature gating, trigger conditions), see [ai/plugin-dev.md](ai/plugin-dev.md).

## Configuration & Documentation

- If a change adds or renames plugin types, config fields, default behaviors, or supported protocols, update `README.md` and `README_EN.md` in the same change when applicable.
- Use `ai/change-impact-matrix.md` to identify required Rust, WebUI, docs, config, packaging, API, and release synchronization.
- When preparing a release, follow the standalone workflow in `ai/release-process.md` for tag-based changelog generation, Cargo version bumps, and release-note updates.
- For the full plugin documentation and WebUI sync checklist (`docs/`, `webui/lib/plugin-definitions/`, `config.yaml`), see [ai/plugin-dev.md](ai/plugin-dev.md).

## Cargo Feature Conventions

See [ai/plugin-dev.md](ai/plugin-dev.md) for the full feature system description, naming rules, the four-step checklist for adding a feature-gated plugin, and the required build verification commands.

## Operations & Maintenance

- Follow `ai/operations-runbook.md` for deployment preflight, health/readiness, diagnosis, reload, upgrade, and rollback procedures.
- Follow `ai/maintenance.md` for dependency updates, toolchain changes, feature hygiene, workspace crate maintenance, and recurring documentation audits.
- Security reports and vulnerability handling follow `SECURITY.md`; do not put secrets or private DNS data into public logs or issues.

## Commit & Pull Request Guidelines

- Use Conventional Commits, for example `feat(cache): add negative cache persistence`.
- Keep commit messages short, action-oriented, and scoped to the subsystem when possible.
- PRs should describe behavior changes, protocol or platform scope, config impact, and the test commands that were run.
- Call out any change that affects the request hot path, default config behavior, or cross-platform support.
