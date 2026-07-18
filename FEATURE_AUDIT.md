# README 功能实现审计（基线）

## 审计范围与判定方法

- 审计日期：2026-07-17
- 审计基线：`b844ea3df4f403a173c2a56f7946fa9c9922e174`
- 声明来源：基线提交中的 `README.md` 与 `README_EN.md`
- 证据范围：本仓库 Rust/WebUI 源码、Cargo feature、配置、测试代码、安装脚本、打包工作流，以及审计时可访问的上游 GitHub Release 元数据

本文回答的是“README 所列能力是否存在可执行的实现路径”，并不把模块名、注释、路线图或仅有配置字段视为实现。判定含义如下：

- **已实现**：存在注册入口和实际执行路径，且能在相应 feature/平台与配置下使用。
- **部分实现**：核心能力存在，但 README 的概括遗漏了会实质影响使用的粒度、协议、平台或兼容性边界。
- **表述错误**：README 的保证与基线代码的实际控制流相反或明显不一致。
- **无法独立验证**：依赖外部仓库、真实设备/网络、历史 Release 或性能环境，单凭本仓库不能给出完整保证。

重要边界：Cargo 默认构建为 `full`，表示相关代码会被编译，不表示所有插件会自动运行。基线 `config.yaml` 的 DNS 处理链只实例化 `forward`、`sequence`、UDP server 和 TCP server；管理 API/WebUI 托管已配置，其他能力均需用户显式配置。`minimal`、`standard`、`full` 的能力也不同，具体以 `Cargo.toml` 为准。

## 总体结论

README 的主体能力不是空谈：DNS 协议栈、策略管线、README 点名的执行器/匹配器/Provider、缓存、上游选择、查询记录、指标、管理 API、WebUI 托管与主要系统联动均有真实代码。

需要纠正或保留边界的项目主要有四类：

1. 完整配置重载会先停止当前 assembly 和全局任务，再装配新配置，因此不是“无中断热重载”；只有 Provider reload 是运行时原位替换。
2. `query_recorder` 能保存 SQLite 历史、筛选、详情和 sequence 级执行事件，但不能从记录中还原某个具体 upstream 为何胜出，也不能解释 `fallback` 的内部决策原因。
3. `adguard_rule`、SOCKS5 和 `ipset`/`nftset` 均有明确兼容性或平台边界，不能按名称推断为全覆盖实现。
4. “高性能”、外部 LuCI 应用在真实 OpenWrt 上的表现、RouterOS/内核 netlink 的真实环境兼容性，以及 fork 自身的历史 Release，无法靠这次静态审计独立保证。

## 逐类审计

### 1. DNS 接入、上游与协议

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| UDP、TCP 入站 | 已实现 | `src/plugin/server/udp.rs`、`src/plugin/server/tcp.rs`；默认配置也实例化了二者。 |
| DoT 入站 | 已实现 | TLS listener 位于 `src/plugin/server/tcp.rs`，由 `server-dot` feature 控制；证书装配见 `src/infra/network/tls_config/server.rs`。 |
| DoQ 入站 | 已实现 | `src/plugin/server/quic.rs`、`src/plugin/server/quic_endpoint.rs`，由 `server-doq` 控制。 |
| DoH 入站 | 已实现 | `src/plugin/server/http/` 提供 HTTP/1.1、HTTP/2 路径；HTTP/3 是单独的 `server-doh3` feature，不应把普通 `server-doh` 等同于 DoH3。 |
| UDP/TCP/DoT/DoQ/DoH 上游 | 已实现 | 构建与连接实现位于 `src/infra/network/upstream/builder.rs`、`src/infra/network/upstream/conn/`；协议分别受 `upstream-*` feature 控制。 |
| 出站 nameserver/bootstrap | 已实现 | `src/infra/network/outbound.rs`、`src/infra/network/resolver/`、`src/infra/network/upstream/bootstrap.rs`。 |
| 多上游并发与结果选择 | 已实现 | `src/plugin/executor/forward/concurrent.rs`、`selection.rs`、`config.rs`；包括并发度和负向响应选择策略。 |
| 主备回退与兜底 | 已实现 | `src/plugin/executor/fallback.rs`；UDP 截断后的 TCP fallback 另见 `src/infra/network/upstream/pooled.rs`。 |
| SOCKS5 统一出站 | 部分实现 | `src/infra/network/proxy.rs` 和 `outbound.rs` 提供 profile；TCP、DoT、非 HTTP/3 的 DoH 可使用代理。UDP、DoQ、DoH3 不使用 SOCKS5，相关 profile 会被拒绝或忽略，见 `src/infra/network/upstream/config.rs`。 |

### 2. 策略编排、匹配与响应处理

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| `server -> DnsContext -> matcher/executor/provider -> upstream` 主链 | 已实现 | `src/core/context.rs`、`src/plugin/registry/`、`src/plugin/executor/sequence/` 和各 server 的 entry 调用共同形成该路径。 |
| 声明式 `sequence` 编排 | 已实现 | `src/plugin/executor/sequence/mod.rs`、`chain.rs` 支持 matcher、executor、`accept`、`reject`、`jump`、`goto`、`mark` 等控制流。 |
| README 点名的 matcher | 已实现 | `src/plugin/matcher/` 中有 `qname`、`question`、`qtype`、`qclass`、`client_ip`、`resp_ip`、`rcode`、`rate_limiter`，并另有 CNAME、响应存在性、mark、env、random 等 matcher。 |
| 按域名、客户端、类型、响应 IP、RCODE 分流 | 已实现 | 对应 matcher 位于 `src/plugin/matcher/`，组合入口位于 `src/plugin/executor/sequence/chain.rs`。 |
| TTL、ECS、响应改写、本地/合成应答 | 已实现 | `src/plugin/executor/ttl.rs`、`ecs_handler.rs`、`redirect.rs`、`hosts.rs`、`arbitrary.rs`、`synthetic_response.rs`。 |
| IPv4/IPv6 偏好 | 已实现 | `src/plugin/executor/dual_selector.rs` 注册 `prefer_ipv4` / `prefer_ipv6`。 |
| A/AAAA IP 主动优选 | 已实现 | `src/plugin/executor/ip_selector/` 实现探测、缓存、预算与选择策略；因此 README 基线路线图把它继续列为待办已过时。 |

### 3. README 点名的执行器

| 分组 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| 转发与容错：`forward`、`cache`、`fallback` | 已实现 | `src/plugin/executor/forward/`、`cache/`、`fallback.rs`。缓存支持 TTL、NXDOMAIN/NODATA 负缓存、lazy cache 与持久化。 |
| 本地与改写：`hosts`、`arbitrary`、`redirect`、`ecs_handler`、`ttl`、`black_hole`、`ip_selector` | 已实现 | 同名模块位于 `src/plugin/executor/`；`arbitrary` 和 `ip_selector` 分别受 `plugin-arbitrary`、`plugin-ip-selector` 控制。 |
| 运维与副作用：`download`、`upgrade`、`reload`、`reload_provider`、`script`、`http_request` | 已实现 | 同名模块位于 `src/plugin/executor/`，重载语义的限制见“配置重载”一节；外部 HTTP/命令能力受相应 feature 与运行权限约束。 |
| 动态规则：`learn_domain` | 已实现 | `src/plugin/executor/learn_domain/` 与 `src/plugin/provider/dynamic_domain_set/` 组成可写入、持久化并即时匹配的动态域名集。 |
| 可观测性：`query_summary`、`query_recorder`、`metrics_collector` | 已实现/部分实现 | `src/plugin/executor/query_summary.rs`、`query_recorder/`、`metrics_collector.rs`；`query_recorder` 的粒度限制见下文。 |

README 列出的执行器均能在基线找到实际 factory/module；其中若 feature 未启用，插件不会出现在该构建的 registry 中，feature 对照见 `Cargo.toml` 与 `tests/feature_gating.rs`。

### 4. Provider 与规则集

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| `domain_set`、`ip_set` | 已实现 | `src/plugin/provider/domain_set.rs`、`ip_set.rs`。 |
| `dynamic_domain_set` | 已实现 | `src/plugin/provider/dynamic_domain_set/`，含存储、后台 flush 和管理 API。 |
| `geoip`、`geosite` | 已实现 | `src/plugin/provider/geoip.rs`、`geosite.rs`，由 `provider-protobuf` 控制。 |
| `adguard_rule` | 部分实现 | `src/plugin/provider/adguard_rule/` 实现常用域名规则、例外、important、`dnstype`、`badfilter` 等；明确跳过 hosts 风格规则、`dnsrewrite`、`client`、`ctag` 与未知 modifier，不能视为完整 AdGuard 规则语法兼容层。 |
| Provider 原位重载 | 已实现 | `src/plugin/executor/reload_provider.rs` 调用当前 `PluginRuntime` 的 provider reload；Provider 使用 snapshot 替换，见 `src/plugin/registry/runtime.rs` 与 Provider 实现。 |

### 5. 查询记录、日志、指标与管理面

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| 实时结构化日志 | 已实现 | `src/infra/observability/logging.rs`、`log_buffer.rs`、`src/api/logs.rs` 与 `webui/components/logs/log-viewer.tsx`。基线中运行日志与查询语义尚未形成用户要求的独立查询日志体系。 |
| 查询记录与检索 | 已实现 | `src/plugin/executor/query_recorder/` 使用 SQLite 后台写入，提供时间范围、qname、qtype、client IP、RCODE、状态、matcher tag 等筛选，以及详情、统计和流式事件 API。 |
| 查询执行路径解释 | 部分实现 | `src/plugin/executor/sequence/chain.rs` 只记录 sequence 级 matcher/executor/builtin 的标签与 outcome；`query_recorder` 从自身所在位置之后开始采集，见 `query_recorder/mod.rs` 的 `step_start_index`。`forward` 的具体 upstream 选择和 `fallback` 的内部原因没有写入该事件模型。 |
| 查询摘要 | 已实现 | `src/plugin/executor/query_summary.rs`。 |
| Prometheus 指标 | 已实现 | 指标注册/导出位于 `src/infra/observability/metrics.rs`、`src/api/metrics.rs`；各主要插件带 `MetricSource`。覆盖度仍可继续扩展，不能理解为每个内部事件均有指标。 |
| 健康检查、配置读取/校验/保存、控制 API | 已实现 | `src/api/health.rs`、`src/api/control.rs`、`src/config/`。 |
| WebUI 静态托管与插件管理界面 | 已实现 | `src/api/static_files.rs`、`src/api/hub.rs`、`webui/app/`、`webui/components/plugins/`。README 也正确说明它不是覆盖所有配置的“一键式完整 DNS 管理面板”。 |
| 管理 API 认证 | 部分实现 | 基线只有可选 HTTP Basic：YAML 中保存用户名/明文密码，服务端逐请求直接比较，见 `src/api/auth.rs`、`src/config/types.rs`。WebUI 的“记住登录”会通过持久化 store 保存密码，见 `webui/lib/auth-store.ts`。它不是账号、会话、OIDC、Passkey 或 TOTP 体系。 |

### 6. 配置重载

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| 完整配置重载 | 已实现，但“无中断”表述错误 | `src/app.rs::handle_reload_command` 先执行 `bootstrap::stop(assembly)` 和 `task::stop_all()`，随后才装配 candidate；失败时再装配旧配置回滚。在 listener/任务重建窗口内不能保证持续服务。 |
| 配置校验失败保留旧配置 | 已实现 | candidate 在停止旧 assembly 前先通过 `load_config_from_path` 解析/校验；装配失败则尝试旧配置回滚，见 `src/app.rs`。但装配阶段失败仍已经发生停服窗口。 |
| Provider 热更新 | 已实现 | `reload_provider` 是当前 runtime 内的原位 reload，与完整应用 reload 不同。 |

因此，准确表述应为“支持应用级完整配置重载和 Provider 原位重载”，而不是“在不中断服务的情况下更新完整配置”。

### 7. 系统联动

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| `ipset`、`nftset` | 部分实现 | Linux 下通过 `crates/ripset/` 的 netlink 后端与 `src/plugin/executor/ipset.rs`、`nftset.rs` 写入；非 Linux 执行器为 no-op/后端为 unsupported。它们属于 `plugin-ipset`，只在 `full` bundle 默认启用。 |
| MikroTik `ros_address_list` | 已实现（单向） | `src/plugin/executor/ros_address_list/` 将 DNS A/AAAA 结果及持久条目同步到 RouterOS，含异步队列、TTL 和清理。README 所说的 DNS 结果同步成立。 |
| MikroTik 双向深度集成 | 未实现的路线图项 | 基线没有从 RouterOS 拉取 address list 的 Provider，也没有通用本地 IP 集双向同步；不能把现有单向 executor 当作该路线图完成。 |
| `reverse_lookup` | 已实现 | `src/plugin/executor/reverse_lookup.rs`，含运行时映射与管理 API。 |

真实内核、权限和 RouterOS 版本兼容性需要在目标系统集成测试，静态代码不能独立证明。

### 8. 构建、部署与发布

| README 能力 | 结论 | 代码证据与边界 |
| --- | --- | --- |
| `minimal` / `standard` / `full` 与按 feature 裁剪 | 已实现 | `Cargo.toml` 已将管理面、协议栈和重插件拆为 feature；`.github/workflows/custom-build.yml` 提供复用构建矩阵。因此“编译定制化”不应继续列为待办。 |
| 多平台 release 构建 | 已实现 | `.github/workflows/release.yml` 包含 Linux GNU/musl、多架构、macOS、Windows、FreeBSD 构建矩阵。 |
| Debian 包 | 已实现 | `Cargo.toml` 的 `package.metadata.deb`、`packaging/` 与 release workflow 的 `cargo deb` 步骤。 |
| 服务化安装/卸载 | 已实现 | `src/infra/service.rs`、`src/cli/service.rs`、`scripts/install.sh`、`install.ps1`、对应 uninstall 脚本。 |
| 独立 WebUI 构建并由管理端口托管 | 已实现 | WebUI CI/build 位于 `.github/workflows/webui-ci.yml` 与 `webui/`；静态文件服务位于 `src/api/static_files.rs`。 |
| OpenWrt LuCI 插件 | 无法仅由本仓库完整验证 | 本仓库安装脚本含 OpenWrt 检测和外部 `luci-app-oxidns` 安装集成，但 LuCI 应用主体位于另一个仓库。基线 README 路线图仍把它列为待办，与同一 README 的下载说明及文档路线图不一致。 |
| 已发布二进制 | 上游可验证，基线 fork 尚无独立发布资产 | 审计时可验证上游 `SvenShi/oxidns` 的 `v1.4.0` Release 具有多平台资产；本审计基线对应的用户 fork 尚无独立 tag 或 Release，不能把上游资产当作 fork 发布能力已经跑通的证据。 |

bundle 边界：`minimal` 不含 API、WebUI、TLS/QUIC；`standard` 含常用加密协议、API/WebUI 和常用插件，但不含 DoH3、MikroTik、`ipset`/`nftset`；`full` 才在 `standard` 上补齐这些能力。

## 表述错误与已修正文案

基线 README 的以下两类保证应当修正：

1. “不中断服务的情况下热更新配置” / “full hot reload”——完整配置 reload 会停止并重建运行组件；应改成“应用级完整配置重载，Provider 可原位重载”。
2. “明确了解选择了哪个上游，以及为什么进入回退路径”——查询记录只提供 sequence 级标签与 outcome；应明确不含具体 upstream 胜出和 fallback 内部决策原因。

README 基线路线图也混入了已经完成的工作：

- 已实现：Cargo 编译定制化、`ip_selector` IP 优选。
- 外部项目已宣称完成且本仓库含安装集成：OpenWrt LuCI 插件；主体仍需到外部仓库/设备验证。
- 尚未实现：RouterOS address list 拉取 Provider、WASM 插件、动态链接库插件。
- 持续演进：插件管理 API、WebUI 覆盖与 Prometheus 指标覆盖。

## 无法独立验证与测试边界

本次结论是基线静态审计，文档修订阶段没有执行本地测试。仓库已有以下测试证据，可供后续 CI 验证：

- `tests/plugin_integration.rs`：配置、registry、sequence 与 live server 集成。
- `tests/feature_gating.rs`：不同 Cargo feature 下的插件可见性。
- `src/infra/network/upstream/tests.rs` 与 `src/plugin/executor/forward/tests.rs`：上游协议配置、代理边界和并发选择。
- `src/plugin/executor/query_recorder/tests.rs`：SQLite 写入、筛选、统计与 execution path。
- `src/plugin/executor/cache/` 内测试：TTL、负缓存、lazy cache 与持久化。
- `crates/ripset/tests/integration.rs`：Linux netlink 集成；需要合适内核与权限。

以下事项不能由静态审计或普通单元测试充分证明：

- “高性能”是相对结论。`benches/` 与 `docs/docs/benchmarks.md` 只能证明项目有自测方法，不能代表所有硬件、规则规模和流量形态。
- 公网 DoT/DoQ/DoH/DoH3 的互操作、证书、代理与网络故障恢复需要真实端到端环境。
- `ipset`、`nftset`、RouterOS、OpenWrt LuCI、服务安装和原地升级需要对应 OS/设备与权限。
- GitHub Actions 工作流存在不等于每个目标当前都能成功构建；应以后续目标仓库 CI 结果为准。
- 发布资产存在不等于所有资产都已逐个启动验证或具备相同运行时能力。

后续二次开发应保留本审计基线，新增功能另以测试、CI 和 Release 资产为证，不应反向改写基线结论。
