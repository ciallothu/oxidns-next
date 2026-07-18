<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/static/img/logo-next-dark.png">
    <img src="docs/static/img/logo-next-light.png" alt="OxiDNS Next" width="128">
  </picture>
</p>

[![oxidns-next downloads](https://img.shields.io/github/downloads/ciallothu/oxidns-next/total)](https://github.com/ciallothu/oxidns-next/releases)
[![Rust CI](https://github.com/ciallothu/oxidns-next/actions/workflows/rust-ci.yml/badge.svg?branch=main)](https://github.com/ciallothu/oxidns-next/actions/workflows/rust-ci.yml)
[![WebUI CI](https://github.com/ciallothu/oxidns-next/actions/workflows/webui-ci.yml/badge.svg)](https://github.com/ciallothu/oxidns-next/actions/workflows/webui-ci.yml)

[中文](README.md) | [English](README_EN.md) · [文档](docs/docs/intro.mdx) · [快速开始](docs/docs/quickstart.mdx) · [插件参考](docs/docs/plugin-reference/overview.md)

# OxiDNS Next

**一个可自建、可检索、可精细分流的 DNS 服务。**

## 产品特点

- 支持 UDP、TCP、DoT、DoQ、DoH，并可为不同域名、客户端和查询结果选择不同处理策略。
- 自带管理 API 与 WebUI，可查看运行状态、查询日志、统计数据和插件配置。
- 支持本地账户、OIDC、通行密钥和 TOTP，适合家庭网络、旁路由、NAS 与 Homelab。
- 查询日志与系统日志分开保存；列表直接显示解析结果，详情提供完整应答与执行流程；持久化存储可选 SQLite、PostgreSQL 或 MySQL。
- 可选 Redis 为 DNS 缓存和查询日志 API 提供共享缓存；Redis 不保存唯一数据，停用或故障时仍可使用本地缓存、上游 DNS 与 SQL 数据库。
- 提供 Linux、macOS、Windows、Docker 和多种路由器架构的发行包。

## 快速开始

Linux 或 macOS：

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sudo sh
```

OpenWrt（便携安装）：

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
```

Windows 管理员 PowerShell：

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.ps1 | iex
```

Docker：

```bash
git clone https://github.com/ciallothu/oxidns-next.git
cd oxidns-next
docker compose up -d
```

默认配置监听 DNS `:5335` 和管理台 `:9199`；仓库 Compose 会把宿主机 DNS `53/udp`、`53/tcp` 映射到容器 `5335`。安装完成后访问 `http://127.0.0.1:9199`；远程首次创建管理员时，请先通过 `OXIDNS_NEXT_BOOTSTRAP_TOKEN` 配置引导令牌。完整安装、服务管理和反向代理说明见[快速开始](docs/docs/quickstart.mdx)。

## 配置怎么写

配置使用 YAML。下面是一个可直接运行的最小完整示例：它开启管理台，把查询记录保存到 SQLite，并将 DNS 请求转发到 `223.5.5.5`。

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

顶层字段负责全局设置，`plugins` 中的每一项都由唯一且不超过 255 个字符的 `tag`、插件 `type` 和可选 `args` 组成。`sequence` 通过 `$tag` 调用其他插件。`query_recorder` 应放在入口 sequence 的第一步，才能记录完整请求与响应。相对路径统一以 `-d/--working-dir` 指定的工作目录为基准。

## 查询日志存储

SQLite 是无需额外服务的默认选项，适合单机快速开始；生产环境首选 PostgreSQL，MySQL 也完整支持：

```yaml
database:
  type: sqlite
  path: "./data/query-log.sqlite"
```

PostgreSQL 和 MySQL 使用连接 URL。生产部署优先选择 PostgreSQL；请通过环境变量提供账号与密码，不要把凭据直接写入配置文件：

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

旧配置中的 `query_recorder.args.path` 仍兼容，等价于 `database.type: sqlite` 与 `database.path`；新配置建议使用显式 `database` 写法。

Redis 是可选缓存层。先配置共享连接，再按需为 `cache` 或 `query_recorder` 启用：

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

`query_recorder` 始终以 SQL 数据库为查询日志的持久化数据源；Redis 连接失败、超时或缓存内容无效时会回退到 SQL。可直接运行的外部数据库与 Redis 部署示例见 [`examples/storage`](examples/storage)。

## 项目定位与设计

> OxiDNS Next 是基于 [上游 OxiDNS](https://github.com/svenshi/oxidns) 的二次开发发行版，由 `ciallothu` 独立维护。项目保留上游作者 Sven Shi 的版权归属，并继续遵循 GPL-3.0-or-later 许可证；OxiDNS Next 与上游项目不是同一发行渠道。

OxiDNS Next 是一个使用 Rust 构建的现代 DNS 引擎，受 [mosdns](https://github.com/IrineSistiana/mosdns) 启发，但不止于规则分流。

它关注的是 DNS 查询在真实网络环境中的完整生命周期：接入、匹配、缓存、转发、回退、改写、本地应答与系统联动，并内置查询记录、Prometheus 指标采集和实时日志能力。

OxiDNS Next 的核心不是“提供更多开关”，而是提供一套清晰、可组合、可调试的策略管线，让你能够用声明式配置描述复杂 DNS 行为。

```text
server -> DnsContext -> matcher / executor / provider -> upstream
```

项目仍在持续开发中，适合需要精细化控制 DNS 行为，并愿意理解其策略模型的用户。

首个版本 `v0.1.0` 在继承上游 DNS 能力的基础上，新增本地登录、OIDC、通行密钥与 TOTP，分离可检索的查询日志，并将仪表盘与插件中心整合为统一工作区。功能声明的实现审计见 [FEATURE_AUDIT.md](FEATURE_AUDIT.md)。

---

## 为什么是 OxiDNS Next

DNS 在复杂网络里往往不只是“查询一个域名”。

你可能需要：

- 根据域名、客户端、查询类型、响应 IP、返回码选择不同上游
- 为不同设备、网段或场景应用不同策略
- 在多个上游之间并发、回退、兜底或按结果决策
- 对响应进行 TTL 调整、ECS 处理、重写或本地应答
- 将 DNS 结果同步到 `ipset`、`nftset` 或 MikroTik RouterOS
- 记录查询过程，并通过日志、查询记录和 Prometheus 插件指标理解系统状态
- 通过应用级重载更新完整配置，并在原位重载 Provider 规则

OxiDNS Next 为这些场景提供的是一套统一的编排模型，而不是分散的功能补丁。

---

## 设计原则

### 可组合

OxiDNS Next 将 DNS 处理过程拆分为 `matcher`、`executor`、`provider` 和 `sequence`。

每个组件只负责一类明确职责，再通过管线组合成完整策略。

### 可调试

DNS 策略一旦复杂，最重要的问题不是“能不能跑”，而是“为什么这样跑”。

OxiDNS Next 提供查询记录（`query_recorder`）、查询摘要统计（`query_summary`）、Prometheus 插件指标（`metrics_collector`）、实时结构化日志和配置校验。WebUI 将可检索的结构化查询历史与系统运行日志分开：查询日志支持按域名或客户端地址关键词及日期范围筛选，系统日志只展示运行事件。查询记录可以展示 `sequence` 中经过的 matcher、executor 及其结果；当前记录粒度不包含具体胜出的 upstream，也不解释 `fallback` 内部选择某一路径的原因。

### 可演进

OxiDNS Next 面向长期运行的自建网络环境设计。

它支持应用级完整配置重载（会重建并重启运行组件）、Provider 原位重载、独立构建的 WebUI 托管，并保留面向插件化和运维能力继续演进的空间。

### 可控

OxiDNS Next 不试图替你隐藏复杂性。

它更适合希望明确掌控 DNS 行为的用户，而不是只想要一个一键安装面板的用户。

---

## 核心能力

| 类别 | 能力 |
| --- | --- |
| 协议 | UDP、TCP、DoT、DoQ、DoH |
| 策略模型 | `sequence`、`matcher`、`executor`、`provider` |
| 执行器 | `forward`、`cache`、`fallback`、`hosts`、`arbitrary`、`redirect`、`ecs_handler`、`ttl`、`black_hole`、`ip_selector`、`download`、`upgrade`、`reload`、`reload_provider`、`script`、`http_request`、`learn_domain`、`query_summary`、`query_recorder`、`metrics_collector` |
| 匹配器 | `qname`、`question`、`qtype`、`qclass`、`client_ip`、`resp_ip`、`rcode`、`rate_limiter` 等 |
| 数据集 | `domain_set`、`dynamic_domain_set`、`ip_set`、`geoip`、`geosite`、`adguard_rule` |
| 出站网络 | `network.outbound` 统一配置 HTTP 下载、升级检查、webhook 与 upstream 使用的 nameservers 与 SOCKS5 |
| 系统联动 | `ipset`、`nftset`、`ros_address_list`、`reverse_lookup` |
| 调试与运维 | 健康检查、配置校验、应用级配置重载、Provider 原位重载、查询记录、Prometheus 插件指标、实时日志 |
| 部署能力 | 多平台构建、Debian 包、OpenWrt 便携部署、独立 WebUI 托管、服务化安装 |

---

## 适合的使用场景

OxiDNS Next 适合部署在需要长期运行、可调试、可扩展的 DNS 环境中。

典型场景包括：

- 家庭网关、旁路由、OpenWrt、NAS、Homelab
- 多上游并发查询、主备回退、协议混合接入
- 可配置并发上游结果选择策略，在速度与负向答案可靠性之间取舍
- 基于域名、客户端、响应结果的精细化策略路由
- DNS 结果驱动的 `ipset` / `nftset` / MikroTik 地址列表同步
- 广告过滤、域名分流、本地覆盖、双栈偏好和 ECS 控制
- 自建可控、可调试的 DNS 基础设施
- 需要通过同一管理端口托管独立 WebUI 的轻量部署

---

## 不适合的场景

OxiDNS Next 不是一个面向所有人的一键 DNS 面板。

如果你主要需要：

- 简单、开箱即用的家庭广告过滤
- 完整的图形化 DNS 管理体验
- 权威 DNS 托管服务
- Kubernetes Service Discovery 插件框架
- 不需要理解配置模型的即装即用工具

那么 AdGuard Home、Pi-hole、Technitium DNS Server 或 CoreDNS 可能更合适。

OxiDNS Next 更适合希望以配置方式明确描述 DNS 行为，并愿意为控制力承担一定复杂度的用户。

---

## 与其他项目的关系

OxiDNS Next 不试图替代所有 DNS 工具：

| 项目 | 更适合的方向 |
| --- | --- |
| AdGuard Home | 开箱即用的家庭广告过滤和 DNS 管理 |
| Pi-hole | 简单、成熟、社区广泛的家庭 DNS 过滤 |
| CoreDNS | 云原生和服务发现插件框架 |
| Technitium DNS Server | 功能完整的通用 DNS 服务器 |
| mosdns | 灵活的 DNS 分流与策略处理 |
| OxiDNS Next | 高性能、可调试、可扩展的 DNS 策略编排引擎 |

---

## 下载

一条命令安装最新 release，并默认注册和启动为系统服务：

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sudo sh
```

Windows 管理员 PowerShell：

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.ps1 | iex
```

默认情况下，Linux / macOS 会安装到 `/opt/oxidns-next`，在 `/usr/local/bin` 创建 `oxidns-next` 命令，并安装、启动系统服务。Windows 会安装到 `%ProgramFiles%\OxiDNS Next`，加入 Machine PATH，并安装、启动系统服务。仅需便携安装时，可设置 `OXIDNS_NEXT_INSTALL_SERVICE=0`，详见快速开始。

OpenWrt 可使用同一脚本进行便携安装；脚本不会注册通用系统服务，也不安装专用 LuCI 插件：

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
# 或：
wget -O- https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/install.sh | sh
```

上游的 [`luci-app-oxidns`](https://github.com/svenshi/luci-app-oxidns) 面向原始 OxiDNS 发行版，不是 OxiDNS Next 的安装器。当前版本如需 OpenWrt 服务托管或 LuCI 页面，请自行完成平台集成。

卸载时默认保留 `config.yaml`：

```bash
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.sh | sudo sh
```

OpenWrt：

```sh
curl -fsSL https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.sh | sh
```

Windows 管理员 PowerShell：

```powershell
irm https://raw.githubusercontent.com/ciallothu/oxidns-next/main/scripts/uninstall.ps1 | iex
```

如果安装时使用了 `sudo` 或自定义 `OXIDNS_NEXT_INSTALL_DIR`，卸载时也请保持相同权限和目录变量。

如果你准备手动下载 GitHub Releases，可按系统选择：

| 系统 / 环境 | 推荐 release 文件 |
| --- | --- |
| Linux x86_64 | `oxidns-next-x86_64-unknown-linux-musl.tar.gz` |
| Linux ARM64 | `oxidns-next-aarch64-unknown-linux-musl.tar.gz` |
| Debian / Ubuntu x86_64 服务安装 | `*_amd64.deb` |
| Debian / Ubuntu ARM64 服务安装 | `*_arm64.deb` |
| OpenWrt | 与设备架构匹配的 Linux musl archive；安装脚本以便携模式部署，不包含专用 LuCI 页面 |
| Alpine Linux x86_64 | `oxidns-next-x86_64-unknown-linux-musl.tar.gz` |
| Alpine Linux ARM64 | `oxidns-next-aarch64-unknown-linux-musl.tar.gz` |
| 32 位 ARM Linux，如部分树莓派 | `oxidns-next-arm-unknown-linux-musleabihf.tar.gz` |
| macOS Intel | `oxidns-next-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `oxidns-next-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `oxidns-next-x86_64-pc-windows-msvc.zip` |
| Windows 32-bit | `oxidns-next-i686-pc-windows-msvc.zip` |
| Windows ARM64 | `oxidns-next-aarch64-pc-windows-msvc.zip` |
| FreeBSD x86_64 | `oxidns-next-x86_64-unknown-freebsd.tar.gz` |

Linux 下如果不确定兼容性，建议优先选择 `musl` 构建。

不确定当前系统和架构时，可执行：

```bash
uname -s && uname -m
```

Windows 可在 PowerShell 中执行：

```powershell
(Get-CimInstance Win32_OperatingSystem).OSArchitecture
```

完整安装流程请参考 [快速开始](docs/docs/quickstart.mdx)。

### 按需裁剪

OxiDNS Next 支持通过 Cargo features 裁剪可选协议和插件。从源码构建时:

```bash
cargo build --release                                                  # 默认 = full
cargo build --release --no-default-features --features minimal         # 最小转发器
cargo build --release --no-default-features --features standard        # 家用 / 路由器
```

公开协议 feature 按能力分层：`resolver-*` 用于 `network.outbound.resolver.nameservers`，`upstream-*` 用于 DNS 上游转发，`server-*` 用于入站服务协议。`standard` 包含常用的 DoT/DoH/DoQ resolver 与 upstream 能力，`full` 额外包含 DoH3。

详见 [自定义编译](docs/docs/custom-build.mdx)。

---

## 文档

- [配置总览](docs/docs/configuration.md)
- [快速开始](docs/docs/quickstart.mdx)
- [OpenWrt 部署](docs/docs/openwrt.mdx)
- [插件总览](docs/docs/plugin-reference/overview.md)
- [管理 API](docs/docs/api.mdx)
- [MikroTik 策略路由](docs/docs/mikrotik-policy-routing.md)
- [常见场景](docs/docs/scenarios.md)
- [架构与设计](docs/docs/architecture-and-design.md)
- [性能与基准](docs/docs/benchmarks.md)
- [路线图](docs/docs/roadmap.md)

---

## 路线图

以下是当前规划。详细说明及已完成的上游历史里程碑请参考[文档路线图](docs/docs/roadmap.md)。

OxiDNS Next `v0.1.0` 已完成独立品牌与发布渠道、多方式管理台认证、查询日志分离，以及仪表盘和插件中心整合。

1. **MikroTik 双向集成**：在现有 DNS 结果单向推送基础上，增加从 RouterOS 拉取地址列表，以及本地 IP 集的双向同步
2. **插件 API、WebUI 与指标增强**：补齐插件运行时管理 API、详情面板和 Prometheus 指标覆盖
3. **简单模式 WebUI**：通过场景模板和表单降低常见家庭 DNS 配置门槛

长期来看，计划探索 WebAssembly 插件和动态链接库插件两种扩展机制，支持第三方开发者独立开发和分发插件。

---

## 状态

OxiDNS Next 仍处于持续开发阶段。

当前版本适合高级用户、测试环境和自建网络场景试用。对于生产环境，请在充分理解配置、日志和回退策略后再部署。

欢迎提交 Issue、反馈真实场景、改进文档或贡献插件。

---

## 免责声明

本项目按"现状"提供，不对其适用性、稳定性或安全性作出保证。

DNS 基础设施直接影响网络可用性、域名解析结果和访问行为。配置错误可能导致断网、DNS 泄漏或解析异常。在生产或关键环境中部署前，请充分理解配置模型、测试回退路径，并做好监控。

项目维护者不对因使用本软件造成的服务中断、数据损失或安全事件承担责任。使用者应自行确保部署和使用方式符合适用的法律法规及第三方服务条款。

---

## 贡献与上游

OxiDNS Next 的问题与改进建议请提交到 [本项目 Issues](https://github.com/ciallothu/oxidns-next/issues)。上游实现、原始版本历史与作者社区请访问 [SvenShi/oxidns](https://github.com/svenshi/oxidns)；请勿将 OxiDNS Next 的问题误报到上游。

---

## 许可证

本项目作为 OxiDNS 的衍生作品，基于 [GNU General Public License v3.0 or later](LICENSE) 开源。原始作者版权声明与许可证文本均予以保留。
