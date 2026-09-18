---
title: Release Notes
sidebar_position: 4
---

import ReleaseCard from '@site/src/components/ReleaseCard';

# Release Notes

This page lists OxiDNS Next releases first. The remaining entries are retained as the historical release record of upstream OxiDNS.

## 2026-09

<div className="release-stack">
  <ReleaseCard version="v0.2.0" badge="Minor Release" date="2026-09-18" defaultOpen>
      **Release Scope**

      - Minor Release. v0.2.0 synchronizes upstream through v1.5.2 while preserving OxiDNS Next accounts, remote SQL storage, Redis caching, branding, and its independent release channel.
      - This release focuses on query-log and latency trends that were limited to short ranges, fragmented chart rendering, and unreliable pointer interaction, while also bringing in runtime-control, RouterOS, rule-loading, and upgrade reliability improvements.

      **Changes**

      - `feat(query_recorder)`: add `day` and true UTC calendar `month` buckets plus explicit `since_ms` / `until_ms` bounds. SQLite, PostgreSQL, and MySQL aggregate every retained row in the selected range instead of sampling the latest 10,000, zero-fill missing buckets, and compute weighted average latency and nearest-rank P95.
      - `feat(webui)`: add 1-hour, 24-hour, 7-day, 30-day, and 1-year query ranges, with 24 hours as the default. The shared ECharts Canvas component supports responsive sizing, light and dark themes, localized tooltips, a bottom zoom control, Ctrl + wheel zoom, drag-to-pan, and mobile gestures.
      - `fix(webui)`: make query trends, latency, dashboard traffic, and system metrics share consistent axes, null handling, tooltips, and resize lifecycle, fixing visual seams, animation jitter, pointer hit offsets, and stale theme rendering.
      - `feat(runtime)`: synchronize trusted ECS client-IP replacement, dedicated dual-stack probes, sequence multi-value mark / `set_mark`, matcher and provider runtime controls, the `response` executor, timezone-aware time matching, and safer streaming loaders for large rule sets.
      - `feat(routeros)` / `fix(upgrade)`: add TLS, queue coalescing, ownership validation, recovery, and cleanup hardening to RouterOS address-list and route synchronization. Downloads use self-cleaning temporary files, with fixes for Windows ZIP upgrades, upgrade status, and runtime lifecycle boundaries.
      - `feat(webui)` / `deps` / `docs`: move the configuration editor to CodeMirror; complete bilingual UI coverage, log formatting, visibility-aware polling, update preferences, and backend-account isolation; update Next.js to 16.3.5 and `rustls` to a patched release, clearing the currently known npm / pnpm and RustSec vulnerabilities; restore the upstream v1.5.x release history and align the release guide with the current artifact contract.

      **Compatibility and Upgrade Notes**

      - The root crate is updated to `0.2.0`, with release tag `v0.2.0`; `oxidns-next-ripset` is updated to `0.1.3`. The already synchronized `oxidns-next-proto 0.1.5` and `oxidns-next-zoneparser 0.1.2` versions remain unchanged.
      - Typical v0.1.1 YAML configurations can upgrade directly; run `oxidns-next check -c <config>` before replacing a production binary. New fields are optional, but user-defined plugin tags must now be safe ASCII path segments, and `qs.exec.*`, `qs.match.*`, and `qs.cron.*` are reserved.
      - Matcher runtime management changed from `/enable` and `/disable` to `/mode`, with `normal`, `always_false`, and `always_true`. Automation using the old endpoints must migrate.
      - New query trend buckets and response bounds are backward-compatible, so existing `minute` and `hour` clients continue to work. The WebUI can only show retained database history; set `retention_days` to at least `366` in advance for a complete yearly view.
      - Deployments using RouterOS plugins should verify comment ownership, TLS, and cleanup policy in a test environment, then validate DNS queries, the management API, query recording, and runtime controls after upgrading.
  </ReleaseCard>
</div>

## 2026-07

<div className="release-stack">
  <ReleaseCard version="v0.1.1" badge="Patch Release" date="2026-07-21">
      **Release Scope**

      - Patch Release. v0.1.1 fixes unreadable chart tooltips in dark mode and removes repeated, reversing, or jittery animations during chart changes and rapid pointer movement.
      - It also synchronizes plugin-topology and Sequence canvases with the active theme, completes the default PostgreSQL / Redis deployment stack, and adds continuous dependency security auditing.

      **Changes**

      - `fix(webui)`: use explicit popover foreground, background, and border colors for query-statistics tooltips, overriding Recharts' black item default and restoring dark-mode readability.
      - `fix(webui)`: disable transitions for query-statistics tooltips, bars, pie slices, and lines so tab or time-range changes, refreshes, and rapid movement between data points no longer replay, reverse, or chase animations.
      - `fix(webui)`: pass the resolved `next-themes` mode to React Flow in the plugin topology and Sequence editor, keeping nodes, controls, and canvases consistent in light and dark modes.
      - `config` / `docker`: make the repository-root configuration use PostgreSQL for query history and enable Redis-backed DNS L2 and query-API caching. The root Compose stack now includes PostgreSQL 17, Redis 7.4, health checks, a persistent PostgreSQL volume, an internal backend network, and an `.env.example` password template.
      - `deps` / `security`: update Next.js, Docusaurus, and dependency resolutions affected by security advisories. Add a Security Audit workflow that runs complete WebUI and documentation dependency audits plus RustSec on relevant pushes and pull requests, weekly, and on demand.
      - `refactor(tls)`: decode TLS certificate and private-key PEM files directly through `rustls-pki-types`, removing the unmaintained direct `rustls-pemfile` dependency without changing certificate paths, key paths, or PEM configuration formats.

      **Compatibility and Upgrade Notes**

      - The root crate version is updated to `0.1.1`, and the release tag is `v0.1.1`. No workspace crate under `crates/` changed, so no child-crate version bump is required.
      - Existing runtime configurations can upgrade directly. There are no new required schema fields, and TLS certificate or private-key configuration does not require migration.
      - The repository-root `config.yaml` deployment defaults have changed and now require `OXIDNS_NEXT_QUERY_DATABASE_URL` and `OXIDNS_NEXT_REDIS_URL`. When using the root Compose stack, copy `.env.example` to `.env`, set the PostgreSQL and Redis passwords, and start the complete stack.
      - SQLite, MySQL, and custom single-container deployments remain supported. Keep the existing deployment configuration instead of replacing it with the new root default. Query history is durable only in SQL; Redis remains disposable cache storage.
      - RustSec retains one assessed exception for `RUSTSEC-2023-0071`: the current OIDC / MySQL dependency paths perform RSA public-key operations, while the advisory concerns private-key timing and no patched release is available.
  </ReleaseCard>

  <ReleaseCard version="v0.1.0" badge="First OxiDNS Next Release" date="2026-07-19">
      **Release Scope**

      - The first public OxiDNS Next release establishes the independent product, release channel, and management console. This rebuilt `v0.1.0` focuses on fixing query-log WebUI stalls and HTTP 504 responses on large databases, while adding PostgreSQL, MySQL, and Redis support.

      **Changes**

      - `feat(query_recorder)`: Query-log persistence now supports SQLite, PostgreSQL, and MySQL. SQLite remains the zero-dependency default; PostgreSQL is the preferred production backend, and MySQL is fully supported as well.
      - `fix(query_recorder)`: Fix contention between record and statistics requests, cancelled SQLite reads continuing to occupy reader capacity, and list requests loading unnecessary full records. Lists now return lightweight summaries, reads have timeout and cancellation boundaries, and the WebUI no longer starts competing first-load requests in parallel.
      - `feat(query_recorder)`: Show actual IP, CNAME, and similar answer values directly in query-history lists while retaining the complete DNS response in details. The execution path is now a compact static flow that follows natural page scrolling instead of intercepting wheel or touch gestures for canvas zoom.
      - `feat(cache)`: Add optional shared Redis caching. The DNS `cache` executor can use Redis as an L2 cache, while `query_recorder` can cache record-list, detail, and statistics API responses.
      - `fix(cache)`: Redis is fail-open. When it is not configured, times out, has a full queue, or is temporarily unavailable, DNS falls back to the in-process cache and upstreams, while query-log APIs fall back to the SQL database. Redis never holds the only copy of query-log data.
      - `feat(auth)` / `feat(webui)`: Add local login, OIDC, passkeys, and TOTP account-security options; give query logs a dedicated view; and combine the dashboard with the plugin center.
      - `ci`: Add dedicated remote storage validation with PostgreSQL 17, MySQL 8.4, and Redis 7.4 service containers.

      **Compatibility and Upgrade Notes**

      - The root crate remains at version `0.1.0`, and the rebuilt release tag is `v0.1.0`.
      - Select `sqlite`, `postgres`, or `mysql` with `query_recorder.args.database.type`. PostgreSQL / MySQL use `url`, `max_connections`, `connect_timeout_ms`, `acquire_timeout_ms`, and `query_timeout_ms`; provide credentials through `${VAR}` environment placeholders.
      - Configure the Redis URL and key prefix under top-level `storage.redis`, then opt in under DNS `cache.args.redis` or `query_recorder.args.api_cache`. Redis is disposable cache storage, not a persistent query-log source.
      - This release replaces the original `v0.1.0`. Existing users should download the rebuilt binaries or pull the `v0.1.0` / `latest` container image again.
      - Existing SQLite configurations continue to work, including the legacy `query_recorder.args.path` form. No SQLite-to-PostgreSQL/MySQL query-log migration tool is provided; confirm that old logs are no longer needed, then start with the new database.
  </ReleaseCard>
</div>

## Upstream OxiDNS release history

## 2026-08

<div className="release-stack">
   <ReleaseCard version="v1.5.2" badge="Patch Release" date="2026-08-18">
       **Release Scope**

       - Patch Release. v1.5.2 focuses on trusted client-IP restoration, isolated dual-stack preference probes, efficient large-rule loading, and safe runtime lifecycles. It also adds sequence mark-set operations and hardens the release pipeline.
       - v1.5.1 YAML configurations upgrade directly. The new `client_ip_from_ecs` executor, `dual_selector.probe_executor` field, and `set_mark` builtin are all opt-in, so existing policies do not change automatically.

       **Changes**

       - `feat(client_ip_from_ecs)`: add an executor that replaces the request-local client IP with an ECS address supplied by a trusted forwarding peer, making that address available to subsequent client-IP matchers, recorders, and policies. Missing or empty allowlists trust IPv4 and IPv6 loopback only, and only complete IPv4 `/32` or IPv6 `/128` host prefixes are accepted. The plugin is included in the standard and full bundles, but not minimal.
       - `feat/fix(dual_selector)`: add optional `probe_executor` support to `prefer_ipv4` and `prefer_ipv6`, allowing preferred-QTYPE probes to use a dedicated `forward` or `sequence`. Omitting it preserves the previous downstream-continuation behavior. Original and probe work use isolated subquery contexts, every return path joins or cancels both tasks, cleanup stops during plugin destruction, and startup rejects missing, wrong-kind, self-referencing, or cyclic dependencies.
       - `feat(sequence)`: let `mark` append multiple `u32` values separated by spaces or commas, and add `set_mark` to replace the entire current mark set. Duplicate values are collapsed, while missing, negative, non-numeric, or overflowing values fail sequence initialization. Dependency graphs, execution paths, and the WebUI editor understand the new syntax.
       - `perf/fix(loaders)`: unify streaming text input and capacity reservation across matchers, hosts, redirect, providers, RouterOS persistence, and zone records, avoiding retention of complete large files or intermediate rule collections. The zone parser gains visitor APIs. Multi-pass compilation fingerprints replayed inputs, publishes only fully built candidates, and moves large compilation work off the async runtime.
       - `fix(runtime/providers)`: provider reload retains serialized ownership after caller cancellation, and runtime teardown drains in-flight reloads and background builds so old snapshot compilation cannot cross reload or destroy boundaries. Replay compilation for AdGuard, V2Ray, and related providers also gains stronger source-location, comment-handling, and rollback coverage.
       - `fix(download/upgrade)`: shared HTTP downloads now own temporary files through drop cleanup, removing incomplete files after timeout, cancellation, or failure and atomically replacing the destination only after success. Windows ZIP-upgrade path handling is corrected as well.
       - `deps/ci/release`: move to `oxidns-mikrotik-rs 0.8.1`, which includes lossless Tokio response delivery, and remove the temporary Git patch plus crates.io `--no-verify`. Update `hotpath`, `base64`, and other dependencies, isolate cross-target build caches, and publish version-bumped workspace support crates in dependency order before the root package.
       - `docs/benchmarks/telegram`: reorganize bilingual installation, configuration, CLI, API, and plugin references; add reproducible multi-implementation benchmark scenarios and results; and render Telegram announcements as compatible HTML that preserves headings, lists, emphasis, inline code, and links, with truncation tests.

       **Compatibility and Upgrade Notes**

       - The root crate version is `1.5.2`; `oxidns-proto` is updated to `0.1.5` and `oxidns-zoneparser` to `0.1.2`; the release tag should be `v1.5.2`. Publication now uploads new support-crate versions before the root crate.
       - v1.5.1 configurations upgrade directly. No fields are renamed or removed, and no existing plugin policy defaults change. Run `oxidns check -c <config-file>` before replacing the binary.
       - `client_ip_from_ecs` changes the request-local client IP observed by later plugins. Place it before the affected matchers and recorders, allow only controlled reverse proxies or local forwarders in `args`, and never trust a source reachable directly by clients. The forwarder must send `/32` or `/128` ECS; network prefixes are ignored.
       - Omitting `dual_selector.probe_executor` preserves v1.5.1 behavior. When configured, probe-context marks, responses, and transient state do not flow back to the original request, but completed external side effects cannot be rolled back; dedicated probe chains should favor side-effect-free resolution executors.
       - Existing single-value `mark` syntax remains valid, and only an explicit `set_mark` clears the previous set. If a large rule file changes during one multi-pass build, the candidate is rejected and the previous snapshot remains active; automation should trigger reload only after the replacement file is fully installed.
   </ReleaseCard>
</div>

## 2026-07

<div className="release-stack">
   <ReleaseCard version="v1.5.1" badge="Patch Release" date="2026-07-22">
       **Release Scope**

       - Patch Release. v1.5.1 focuses on matcher runtime control, upgrade operations, and WebUI quality. It expands temporary matcher switching into tri-state base-result controls, adds force and post-upgrade cleanup controls, and delivers a concentrated set of localization, polling, log-viewer, and plugin-card fixes.
       - Existing YAML configurations upgrade directly, but the matcher runtime management API has a breaking change. Clients using that API must migrate before upgrading.

       **Changes**

       - `feat/fix(matcher)`: replace the runtime switch with `normal`, `always_false`, and `always_true` modes. Both fixed modes skip the matcher implementation and fix its base Boolean value; each `$tag` or `!$tag` reference then applies its own outer negation, so positive and negated results remain opposites. `sequence`, `any_match`, and query recorder now track both the fixed mode and effective match result, with regression coverage for shared controls.
       - `feat(upgrade)`: let the management API and WebUI set `force` to reinstall a release even when it is already current. Add `cleanup` to control removal of download caches and backups after a successful upgrade. The WebUI persists both preferences and generates equivalent CLI commands. Cleanup releases the upgrade lock first and reports cleanup failures without changing a successful apply result.
       - `fix(webui/i18n)`: complete Chinese and English localization for RouterOS, plugin definitions, metrics, configuration history, and console components, including locale-aware date formatting. Add coverage auditing to prevent missing English translations or fallback to Chinese.
       - `fix(webui/runtime)`: schedule runtime polling according to page visibility while retaining background metric collection. Isolate responses, metric baselines, and update-check caches between backend connections; fetch matcher state only on initial load or explicit refresh; reset QPS sampling after long gaps.
       - `feat(webui/logs)`: add persisted timestamp formats, optional elapsed-time display, adaptive duration units, and compact target paths. Unify plugin configuration and metric cards around an adaptive grid, with better RouterOS write-result, timestamp-metric, and system-memory presentation.
       - `perf(build)`: use size-oriented release optimization, fat LTO, one codegen unit, and symbol stripping. Limit Tokio, QUIC, and TLS dependencies to required features, and attempt UPX compression for minimal and standard release artifacts without making compression failure block publication. Exclude development-only benchmarks, site documentation, and WebUI sources from the crates.io source package to stay clear of the registry size limit.
       - `deps/ci/release`: update `wincode`, `syn`, other Rust dependencies, and GitHub Actions. Temporarily apply the RouterOS unbounded-response-channel fix through a Git patch to prevent protocol-event loss under burst traffic. The patch is not published separately, and crates.io publication uses `--no-verify` until upstream ships the fix. GitHub Release and Telegram announcements now share version-heading-validated release notes; announcements target the configured topic and are pinned automatically.

       **Compatibility and Upgrade Notes**

       - The root crate version is `1.5.1`; publishable workspace crate versions remain unchanged, and the release tag should be `v1.5.1`. The RouterOS patch is not published as a separate crate.
       - v1.5.0 YAML configurations upgrade directly. No configuration fields are added, renamed, or given new defaults. Run `oxidns check -c <config-file>` before replacing the binary.
       - **Matcher API migration**: `POST /api/plugins/<matcher_tag>/enable` and `/disable` are removed. Use `POST /api/plugins/<matcher_tag>/mode` with `{ "mode": "normal|always_false|always_true" }`. The `GET /status` response replaces `enabled` with `mode`. Unmigrated automation and third-party controllers will receive a 404 or fail response decoding.
       - Fixed matcher modes exist only in the current runtime and reset to `normal` after an application reload or process restart. The mode is shared by matcher tag, while reference semantics remain local: with `always_false`, `$tag` misses and `!$tag` matches; `always_true` produces the opposite results. Control the `any_match` matcher itself when the whole composition must be fixed.
       - The WebUI defaults to deleting download caches and backups after a successful upgrade. Disable post-upgrade cleanup when local rollback files must be retained. Use `force` only to repair a damaged installation or redeploy the same version, after confirming the intended bundle and platform.
       - Minimal and standard artifacts may be UPX-compressed. Environments with binary scanners, allowlists, or integrity baselines should verify the release asset digest again and smoke-test startup, upgrade, and rollback before production replacement.
   </ReleaseCard>

   <ReleaseCard version="v1.5.0" badge="Minor Release" date="2026-07-19">
       **Release Scope**

       - Minor Release. v1.5.0 centers on RouterOS policy synchronization and live operations: it adds the `ros_route` static policy-route plugin, comprehensively rebuilds `ros_address_list`, and adds management API plus WebUI runtime controls for matchers and providers.
       - It also adds a configurable `response` executor, timezone-aware `time` matching, query-recorder space reclamation, stronger CNAME response selection, advanced WebUI configuration, and broader OpenWrt, ARMv7, and container delivery support.

       **Changes**

       - `feat(routeros)`: add `ros_route` to synchronize observed DNS A/AAAA addresses into per-IP static routes in a selected RouterOS routing table, with IPv4/IPv6 gateways, distance, TTL leases, persistent IP/CIDR entries, optional conntrack-delayed removal, startup recovery, and bounded shutdown cleanup.
       - `refactor(routeros)`: share TLS/API-SSL transport, bounded parallel batching, key-coalescing queues, leases, reconciliation, retry, and lifecycle primitives across `ros_address_list` and `ros_route`. RouterOS outages no longer block DNS startup; synchronous mode is limited by `wait_timeout` and never changes the DNS response on failure; deletion revalidates internal ID, target key, and ownership comment to avoid touching foreign entries.
       - `feat(runtime_control)`: all matchers gain live status, enable, and disable controls; providers gain serialized reload with conflicting concurrent requests rejected. The management API, logs, and WebUI plugin detail panels expose these controls, while builds without the API feature keep them disabled.
       - `feat(response/matcher)`: add a `response` executor that builds Answer, Authority, and Additional sections from zone-record templates with RCODE, flags, and `{qname}`/`{qclass}` placeholders. The `time` matcher gains IANA timezones, multiple periods, overnight windows, weekdays, and month-day constraints.
       - `refactor(query_recorder)`: coordinate readers, writers, and maintenance sharing one SQLite database. Retention cleanup and manual history clearing now delete in batches, truncate WAL, migrate legacy databases to incremental auto-vacuum, reclaim disk space, and keep the in-memory tail consistent with stored results.
       - `fix(dns/forward/cache)`: add a shared query-aware response classifier. Concurrent upstream selection distinguishes complete positives, definitive negatives, and incomplete aliases; bare CNAME responses no longer win early, vote as negatives, or populate address caches. Cache dump/load, lazy refresh, TTL, and persisted-age behavior are hardened while repeated CNAME scans are reduced.
       - `feat(webui)`: add collapsible advanced configuration fields while preserving explicit defaults, `false`, `0`, and empty objects. Replace Monaco with a locally hosted CodeMirror YAML editor with stronger validation, completion, array, and time-period serialization. Improve DNS traffic, process memory, and unavailable-metrics dashboard states.
       - `feat(release/operations)`: add an ARMv7 release target; let the installer deploy `luci-app-oxidns` and its Chinese package on OpenWrt; move the container to an Alpine build stage plus BusyBox musl runtime; split upgrade discovery, digest verification, archive handling, and binary/WebUI installation while correcting full/slim target selection.
       - `fix(config/api/health)`: reject path-unsafe plugin tags and the reserved quick-setup namespace, consistently URL-encode plugin API routes, and report health only after plugin initialization. Synchronize new `response` and RouterOS configuration, features, and build-info capabilities.
       - `deps/ci/docs`: update dependencies and GitHub Actions, including the `oxidns-proto` nightly-Clippy fix. Clarify server, plugin, infra, upgrade, and provider/V2Ray module boundaries and update bilingual API, plugin, OpenWrt, installation, and operations documentation.

       **Compatibility and Upgrade Notes**

       - The root crate version is `1.5.0`; `oxidns-proto` is updated to `0.1.4`; the release tag should be `v1.5.0`.
       - Most v1.4.0 configurations upgrade directly; all newly introduced capabilities are optional. Run `oxidns check` before upgrading. Unsafe plugin tags and reserved quick-setup tags now fail validation and must be renamed together with all references.
       - **RouterOS ownership migration**: the default `ros_address_list.comment_prefix` changes from `fdns` to `oxi`. To continue recognizing, refreshing, or cleaning entries created by older releases, explicitly keep `comment_prefix: fdns` in the existing plugin configuration; handle the old namespace before switching to `oxi`.
       - `ros_address_list` adds optional `tls`, `wait_timeout`, and `queue_capacity`; existing address-list, persistent, and TTL settings remain valid. `cleanup_on_shutdown` still defaults to `true`, and application reload shuts down the old instance first. Deployments requiring policy continuity should evaluate `false` and must not run two processes with the same tag, comment prefix, and target list.
       - `ros_route` is new and requires a pre-created RouterOS routing table/rule plus at least one gateway. `fixed_ttl: 0` creates dynamic routes that never expire naturally, and dynamic leases have no entry-count cap; assess RouterOS routing-table and OxiDNS memory capacity first. Custom builds can select `plugin-ros-address-list` or `plugin-ros-route`; `plugin-mikrotik` remains the aggregate feature.
       - Legacy query-recorder databases migrate to incremental auto-vacuum on the first retention cleanup or manual history clear. The first migration/reclaim of a large database may create noticeable disk I/O, so schedule it outside peak traffic and keep free space available.
       - `forward.concurrent: 1` still does not retry upstreams that were not started. An incomplete CNAME response can still be returned when no better result exists, but is not cached as an address answer. Legacy CNAME-only address entries in cache dumps are discarded during load or hit validation.
       - The container now uses a musl/BusyBox runtime while retaining CA and timezone data. Container, OpenWrt, and ARMv7 deployments should verify startup arguments, mounts, timezone behavior, and upgrade/rollback procedures before production replacement.
   </ReleaseCard>
</div>

## Archive

Detailed notes are partitioned by month so the current page remains bounded:

- [June 2026](releases/2026-06.md)
- [May 2026](releases/2026-05.md)
- [April 2026](releases/2026-04.md)
- [March 2026](releases/2026-03.md)

Before upgrading, read the target release and every intervening “Configuration and Upgrade Notes” section. Historical notes describe behavior at that time; current parameters come from the code and documentation at the matching release tag.
