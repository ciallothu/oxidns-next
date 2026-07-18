<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/static/img/logo-next-dark.png">
    <img src="docs/static/img/logo-next-light.png" alt="OxiDNS Next" width="128">
  </picture>
</p>

[![oxidns-next downloads](https://img.shields.io/github/downloads/ciallothu/oxidns-next/total)](https://github.com/ciallothu/oxidns-next/releases)
[![Rust CI](https://github.com/ciallothu/oxidns-next/actions/workflows/rust-ci.yml/badge.svg?branch=main)](https://github.com/ciallothu/oxidns-next/actions/workflows/rust-ci.yml)
[![WebUI CI](https://github.com/ciallothu/oxidns-next/actions/workflows/webui-ci.yml/badge.svg)](https://github.com/ciallothu/oxidns-next/actions/workflows/webui-ci.yml)

[中文](README.md) | [English](README_EN.md) · [Documentation](docs/i18n/en/docusaurus-plugin-content-docs/current/intro.mdx) · [Quick Start](docs/i18n/en/docusaurus-plugin-content-docs/current/quickstart.mdx) · [Plugin Reference](docs/i18n/en/docusaurus-plugin-content-docs/current/plugin-reference/overview.md)

# OxiDNS Next

**A self-hosted DNS service with searchable history and precise routing controls.**

## Product Highlights

- Serves UDP, TCP, DoT, DoQ, and DoH, with configurable handling for different domains, clients, and query results.
- Includes a management API and WebUI for runtime status, query history, statistics, and plugin configuration.
- Supports local accounts, OIDC, passkeys, and TOTP for home networks, routers, NAS devices, and homelabs.
- Keeps query history separate from system logs. Lists show resolved answers directly, details show the complete response and execution flow, and persistence can use SQLite, PostgreSQL, or MySQL.
- Can use Redis as a shared cache for DNS answers and query-log APIs. Redis is never the sole data store; DNS and query-log access continue through local cache, upstream DNS, and SQL when Redis is unavailable.
- Ships release packages for Linux, macOS, Windows, Docker, and multiple router architectures.

## Quick Start

Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sudo sh
```

OpenWrt (portable install):

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
```

Elevated Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.ps1 | iex
```

Docker:

```bash
git clone https://github.com/ciallothu/oxidns-next.git
cd oxidns-next
docker compose up -d
```

The default configuration listens for DNS on `:5335` and serves the console on `:9199`; the repository Compose file maps host `53/udp` and `53/tcp` to container port `5335`. Open `http://127.0.0.1:9199` after installation. For remote administrator bootstrap, configure `OXIDNS_NEXT_BOOTSTRAP_TOKEN` first. See [Quick Start](docs/i18n/en/docusaurus-plugin-content-docs/current/quickstart.mdx) for installation, service, and reverse-proxy details.

## Configuration Basics

Configuration is written in YAML. This minimal complete example enables the console, records queries in SQLite, and forwards DNS requests to `223.5.5.5`:

```yaml
log:
  level: info

api:
  http:
    listen: ":9199"
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      # bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
    webui:
      root: "./webui"

plugins:
  - tag: query_log
    type: query_recorder
    args:
      database:
        type: sqlite
        path: "./data/query-log.sqlite"
      retention_days: 7

  - tag: forward
    type: forward
    args:
      upstreams:
        - addr: "223.5.5.5"

  - tag: main_sequence
    type: sequence
    args:
      - exec: $query_log
      - exec: $forward
      - exec: accept

  - tag: udp_server
    type: udp_server
    args:
      entry: main_sequence
      listen: ":5335"

  - tag: tcp_server
    type: tcp_server
    args:
      entry: main_sequence
      listen: ":5335"
```

Top-level fields configure the application. Every item under `plugins` has a unique `tag` of at most 255 characters, a plugin `type`, and optional `args`. A `sequence` calls another plugin with `$tag`. Put `query_recorder` first in an entry sequence to capture the complete request and response. Relative paths use the directory selected by `-d/--working-dir`.

## Query-History Storage

SQLite is the zero-dependency default for a quick single-node start. PostgreSQL is the preferred production database, with MySQL fully supported as well:

```yaml
database:
  type: sqlite
  path: "./data/query-log.sqlite"
```

PostgreSQL and MySQL use connection URLs. Prefer PostgreSQL for production, and supply credentials through environment variables instead of writing them directly into the configuration file:

```yaml
# PostgreSQL
database:
  type: postgres
  url: "${OXIDNS_NEXT_QUERY_DATABASE_URL}"
  max_connections: 8
  connect_timeout_ms: 5000
  acquire_timeout_ms: 3000
  query_timeout_ms: 20000
```

```yaml
# MySQL
database:
  type: mysql
  url: "${OXIDNS_NEXT_QUERY_DATABASE_URL}"
  max_connections: 8
  connect_timeout_ms: 5000
  acquire_timeout_ms: 3000
  query_timeout_ms: 20000
```

The legacy `query_recorder.args.path` field remains supported and is equivalent to `database.type: sqlite` plus `database.path`. New configurations should use the explicit `database` form.

Redis is an optional cache layer. Define the shared connection once, then enable it for `cache` or `query_recorder` as needed:

```yaml
storage:
  redis:
    url: "${OXIDNS_NEXT_REDIS_URL}"
    key_prefix: "oxidns-next"
    connect_timeout_ms: 1000

plugins:
  - tag: dns_cache
    type: cache
    args:
      redis:
        enabled: true
        command_timeout_ms: 20
        max_inflight: 64
        write_queue_size: 4096
        failure_threshold: 3
        retry_after_ms: 30000

  - tag: query_log
    type: query_recorder
    args:
      database:
        type: sqlite
        path: "./data/query-log.sqlite"
      api_cache:
        enabled: true
        records_ttl_ms: 2000
        stats_ttl_ms: 5000
        command_timeout_ms: 100
        max_value_bytes: 1048576
```

The SQL database always remains the durable source for `query_recorder`. If Redis is unavailable, times out, or contains an invalid cached value, query APIs fall back to SQL. Runnable external-database and Redis deployment examples are under [`examples/storage`](examples/storage).

## Positioning and Design

> OxiDNS Next is an independently maintained derivative of [upstream OxiDNS](https://github.com/svenshi/oxidns), maintained by `ciallothu`. It retains Sven Shi's upstream copyright and remains licensed under GPL-3.0-or-later. OxiDNS Next and upstream OxiDNS use separate release channels.

OxiDNS Next is a modern DNS engine built with Rust. It is inspired by [mosdns](https://github.com/IrineSistiana/mosdns), but it is not merely another rule-based DNS forwarder.

It focuses on the full lifecycle of DNS queries in real-world network environments: ingress, matching, caching, forwarding, fallback, rewriting, local answers, and system integrations, with built-in query recording, Prometheus metrics collection, and real-time logging.

The core idea of OxiDNS Next is not to expose more switches. It is to provide a clear, composable, and debuggable policy pipeline that lets you describe complex DNS behavior through declarative configuration.

```text
server -> DnsContext -> matcher / executor / provider -> upstream
```

The project is under active development. It is designed for users who need fine-grained control over DNS behavior and are willing to understand its policy model.

The first `v0.1.0` release inherits the upstream DNS engine and adds local login, OIDC, passkeys, and TOTP; separates searchable query logs; and combines the dashboard and plugin center into one workspace. See [FEATURE_AUDIT.md](FEATURE_AUDIT.md) for the implementation audit behind the capability claims.

---

## Why OxiDNS Next

In complex networks, DNS is often more than “resolve this domain”.

You may need to:

- Select different upstreams based on domain, client, query type, response IP, or response code
- Apply different policies to different devices, subnets, or scenarios
- Race, fallback, fail over, or make decisions based on upstream results
- Adjust TTL, handle ECS, rewrite responses, or return local answers
- Sync DNS results into `ipset`, `nftset`, or MikroTik RouterOS
- Record query behavior and understand system state through logs, query records, and Prometheus plugin metrics
- Reload the complete application configuration and reload provider rules in place

OxiDNS Next provides a unified orchestration model for these scenarios instead of a collection of isolated feature switches.

---

## Design Principles

### Composable

OxiDNS Next decomposes DNS processing into `matcher`, `executor`, `provider`, and `sequence`.

Each component has a focused responsibility, and complete policies are built by composing them into pipelines.

### Debuggable

Once DNS policies become complex, the most important question is not just “does it run”, but “why did it behave this way”.

OxiDNS Next provides query recording (`query_recorder`), query summary statistics (`query_summary`), Prometheus plugin metrics (`metrics_collector`), real-time structured logging, and configuration validation. The WebUI keeps searchable structured query history separate from runtime system logs: query history can be filtered by domain or client-address keywords and date range, while system logs show runtime events only. Query records show the matchers, executors, and outcomes observed in a `sequence`; they do not currently identify the winning upstream or explain an internal `fallback` branch decision.

### Evolvable

OxiDNS Next is designed for long-running self-hosted network environments.

It supports application-level configuration reloads (which rebuild and restart runtime components), in-place provider reloads, separately built WebUI hosting, and keeps room for future plugin and operations-oriented improvements.

### Explicit

OxiDNS Next does not try to hide complexity from you.

It is better suited for users who want explicit control over DNS behavior, rather than users who only want a one-click DNS dashboard.

---

## Core Capabilities

| Category | Capabilities |
| --- | --- |
| Protocols | UDP, TCP, DoT, DoQ, DoH |
| Policy model | `sequence`, `matcher`, `executor`, `provider` |
| Executors | `forward`, `cache`, `fallback`, `hosts`, `arbitrary`, `redirect`, `ecs_handler`, `ttl`, `black_hole`, `ip_selector`, `download`, `upgrade`, `reload`, `reload_provider`, `script`, `http_request`, `learn_domain`, `query_summary`, `query_recorder`, `metrics_collector` |
| Matchers | `qname`, `question`, `qtype`, `qclass`, `client_ip`, `resp_ip`, `rcode`, `rate_limiter`, and more |
| Data sets | `domain_set`, `dynamic_domain_set`, `ip_set`, `geoip`, `geosite`, `adguard_rule` |
| Outbound networking | `network.outbound` centralizes nameservers and SOCKS5 settings for HTTP downloads, upgrade checks, webhooks, and upstreams |
| System integrations | `ipset`, `nftset`, `ros_address_list`, `reverse_lookup` |
| Debugging and operations | Health checks, config validation, application-level config reload, in-place provider reload, query records, Prometheus plugin metrics, real-time logs |
| Deployment | Multi-platform builds, Debian packages, portable OpenWrt deployment, standalone WebUI hosting, service installation |

---

## Good Fits

OxiDNS Next is a good fit for DNS environments that need to be long-running, debuggable, and extensible.

Typical use cases include:

- Home gateways, side routers, OpenWrt, NAS, and homelab setups
- Multi-upstream racing, fallback chains, and mixed protocol environments
- Configurable concurrent upstream response selection to balance latency and negative-answer confidence
- Fine-grained DNS policy routing based on domains, clients, and response results
- DNS-result-driven `ipset` / `nftset` / MikroTik address list synchronization
- Ad filtering, domain routing, local overrides, dual-stack preferences, and ECS control
- Self-hosted DNS infrastructure that needs explicit control and debugging
- Lightweight deployments that serve a separately built WebUI on the same management port

---

## Non-Goals

OxiDNS Next is not a one-click DNS dashboard for everyone.

If you primarily need:

- Simple and ready-to-use home ad blocking
- A full graphical DNS management experience
- Authoritative DNS hosting
- A Kubernetes service discovery plugin framework
- A zero-configuration tool that does not require understanding its configuration model

Then AdGuard Home, Pi-hole, Technitium DNS Server, or CoreDNS may be a better fit.

OxiDNS Next is for users who want to describe DNS behavior explicitly through configuration and are willing to accept some complexity in exchange for control.

---

## Relationship to Other Projects

OxiDNS Next does not try to replace every DNS tool:

| Project | Best suited for |
| --- | --- |
| AdGuard Home | Ready-to-use home ad blocking and DNS management |
| Pi-hole | Simple, mature, community-proven home DNS filtering |
| CoreDNS | Cloud-native DNS and service discovery plugin framework |
| Technitium DNS Server | Full-featured general-purpose DNS server |
| mosdns | Flexible DNS routing and policy processing |
| OxiDNS Next | High-performance, debuggable, extensible DNS policy orchestration |

---

## Download

Install the latest release with one command. By default this installs and starts OxiDNS Next as a system service:

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sudo sh
```

Elevated Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.ps1 | iex
```

By default, Linux / macOS installs into `/opt/oxidns-next`, creates `/usr/local/bin/oxidns-next`, and installs and starts the system service. Windows installs into `%ProgramFiles%\OxiDNS Next`, adds it to the Machine PATH, and installs and starts the service. For a portable user install, set `OXIDNS_NEXT_INSTALL_SERVICE=0`; see Quick Start for details.

OpenWrt can use the same script for a portable install. The script does not register a generic service or install a dedicated LuCI app:

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
# or:
wget -O- https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
```

Upstream [`luci-app-oxidns`](https://github.com/svenshi/luci-app-oxidns) targets the original OxiDNS distribution and is not an OxiDNS Next installer. For this release, OpenWrt service management or LuCI pages require your own platform integration.

Uninstall while keeping `config.yaml`:

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.sh | sudo sh
```

OpenWrt:

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.sh | sh
```

Elevated Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.ps1 | iex
```

If you installed with `sudo` or a custom `OXIDNS_NEXT_INSTALL_DIR`, use the same privilege level and directory variable when uninstalling.

If you want to download a GitHub release directly, use this platform guide:

| System / Environment | Recommended release asset |
| --- | --- |
| Linux x86_64 | `oxidns-next-x86_64-unknown-linux-musl.tar.gz` |
| Linux ARM64 | `oxidns-next-aarch64-unknown-linux-musl.tar.gz` |
| Debian / Ubuntu x86_64 service install | `*_amd64.deb` |
| Debian / Ubuntu ARM64 service install | `*_arm64.deb` |
| OpenWrt | A Linux musl archive matching the device architecture; the installer deploys it portably without dedicated LuCI pages |
| Alpine Linux x86_64 | `oxidns-next-x86_64-unknown-linux-musl.tar.gz` |
| Alpine Linux ARM64 | `oxidns-next-aarch64-unknown-linux-musl.tar.gz` |
| 32-bit ARM Linux, including some Raspberry Pi installs | `oxidns-next-arm-unknown-linux-musleabihf.tar.gz` |
| macOS Intel | `oxidns-next-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `oxidns-next-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `oxidns-next-x86_64-pc-windows-msvc.zip` |
| Windows 32-bit | `oxidns-next-i686-pc-windows-msvc.zip` |
| Windows ARM64 | `oxidns-next-aarch64-pc-windows-msvc.zip` |
| FreeBSD x86_64 | `oxidns-next-x86_64-unknown-freebsd.tar.gz` |

On Linux, prefer the `musl` build if you are unsure about compatibility.

If you are unsure which platform you are on, run:

```bash
uname -s && uname -m
```

On Windows PowerShell, run:

```powershell
(Get-CimInstance Win32_OperatingSystem).OSArchitecture
```

For the full installation flow, see [Quick Start](docs/i18n/en/docusaurus-plugin-content-docs/current/quickstart.mdx).

### Slim builds

OxiDNS Next lets you strip optional protocols and plugins via Cargo features. When building from source:

```bash
cargo build --release                                                  # default = full
cargo build --release --no-default-features --features minimal         # bare forwarder
cargo build --release --no-default-features --features standard        # home / router
```

Public protocol features are grouped by product capability: `resolver-*` enables `network.outbound.resolver.nameservers`, `upstream-*` enables DNS upstream forwarding, and `server-*` enables inbound serving protocols. `standard` includes the common DoT/DoH/DoQ resolver and upstream capabilities; `full` adds DoH3.

See [Custom Build](docs/i18n/en/docusaurus-plugin-content-docs/current/custom-build.mdx) for details.

---

## Documentation

- [Configuration](docs/i18n/en/docusaurus-plugin-content-docs/current/configuration.md)
- [Quick Start](docs/i18n/en/docusaurus-plugin-content-docs/current/quickstart.mdx)
- [OpenWrt deployment](docs/i18n/en/docusaurus-plugin-content-docs/current/openwrt.mdx)
- [Plugin Overview](docs/i18n/en/docusaurus-plugin-content-docs/current/plugin-reference/overview.md)
- [Management API](docs/i18n/en/docusaurus-plugin-content-docs/current/api.mdx)
- [MikroTik Policy Routing](docs/i18n/en/docusaurus-plugin-content-docs/current/mikrotik-policy-routing.md)
- [Common Scenarios](docs/i18n/en/docusaurus-plugin-content-docs/current/scenarios.md)
- [Architecture and Design](docs/i18n/en/docusaurus-plugin-content-docs/current/architecture-and-design.md)
- [Performance and Benchmarks](docs/i18n/en/docusaurus-plugin-content-docs/current/benchmarks.md)
- [Roadmap](docs/i18n/en/docusaurus-plugin-content-docs/current/roadmap.md)

---

## Roadmap

The following items remain planned. See the [documentation roadmap](docs/i18n/en/docusaurus-plugin-content-docs/current/roadmap.md) for full details and completed upstream milestones.

OxiDNS Next `v0.1.0` established the independent identity and release channel, multiple console authentication methods, separate query logs, and the combined dashboard and plugin workspace.

1. **Bidirectional MikroTik integration**: Build on the existing one-way DNS-result push with RouterOS address-list imports and two-way local IP-set synchronization
2. **Plugin APIs, WebUI, and metrics**: Complete runtime management APIs, detail panels, and Prometheus coverage for more plugins
3. **Simple-mode WebUI**: Use scenario templates and forms to lower the setup barrier for common home DNS configurations

Looking further ahead, two plugin extension mechanisms are planned: WebAssembly plugins and dynamic library plugins, enabling third-party developers to build and distribute plugins independently.

---

## Project Status

OxiDNS Next is under active development.

The current version is suitable for advanced users, testing environments, and self-hosted network setups. For production use, make sure you understand the configuration, logs, and fallback behavior before deploying it.

Issues, real-world feedback, documentation improvements, and plugin contributions are welcome.

---

## Disclaimer

This project is provided as-is, without warranties of any kind.

DNS infrastructure directly affects network availability, name resolution results, and access behavior. Misconfiguration can cause connectivity loss, DNS leaks, or unexpected resolution failures. Before deploying in production or critical environments, make sure you understand the configuration model, have tested fallback paths, and have monitoring in place.

The maintainers are not responsible for any service disruption, data loss, or security incident resulting from the use of this software. Users are responsible for ensuring their deployment and usage comply with applicable laws, regulations, and third-party service terms.

---

## Contributing and Upstream

Please report OxiDNS Next issues and proposals in [this project's issue tracker](https://github.com/ciallothu/oxidns-next/issues). For the original implementation, upstream release history, and upstream community, visit [SvenShi/oxidns](https://github.com/svenshi/oxidns). Do not report OxiDNS Next-specific problems to upstream.

---

## License

As a derivative of OxiDNS, this project is licensed under the [GNU General Public License v3.0 or later](LICENSE). The original copyright notices and license text are retained.
