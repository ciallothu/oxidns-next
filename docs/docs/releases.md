---
title: 版本更新
sidebar_position: 4
---

import ReleaseCard from '@site/src/components/ReleaseCard';

# 版本更新

本页首先记录 OxiDNS Next 的版本；后续内容保留为上游 OxiDNS 的历史版本记录。

## 2026-09

<div className="release-stack">
  <ReleaseCard version="v0.2.0" badge="Minor Release" date="2026-09-18" defaultOpen>
      **版本定位**

      - Minor Release。v0.2.0 同步上游至 v1.5.2，并保留 OxiDNS Next 的账户、远程 SQL 存储、Redis 缓存、品牌和独立发布链路。
      - 本版本重点修复查询日志与延时趋势只能查看短时间范围、图表渲染割裂和鼠标交互不稳定的问题，同时带入运行时控制、RouterOS、规则加载与升级可靠性改进。

      **主要变更**

      - `feat(query_recorder)`：趋势 API 新增 `day` 与真实 UTC 日历 `month` 桶，支持明确的 `since_ms` / `until_ms` 边界；SQLite、PostgreSQL、MySQL 均聚合所选保留范围内的全部记录，不再受最近 10,000 条采样限制，并补齐空桶、计算加权平均延时和最近秩 P95。
      - `feat(webui)`：查询统计提供 1 小时、24 小时、7 天、30 天、1 年五档范围，默认 24 小时。统一的 ECharts Canvas 组件支持响应式尺寸、深浅色主题、本地化 Tooltip、底部缩放、Ctrl + 滚轮缩放、拖动平移和移动端手势。
      - `fix(webui)`：查询趋势、延时、仪表盘流量和系统指标共用一致的坐标、空值、Tooltip 与 resize 生命周期，修复旧图表的视觉割裂、动画抖动、鼠标命中偏移和主题切换残留。
      - `feat(runtime)`：同步可信 ECS 客户端 IP、双栈专用探针、sequence 多值 mark / `set_mark`、matcher/provider 运行时控制、`response` 执行器、时区感知时间匹配和更安全的大规则集流式加载。
      - `feat(routeros)` / `fix(upgrade)`：RouterOS address-list 与 route 同步获得 TLS、队列合并、所有权校验、恢复与清理加固；升级下载使用可清理临时文件，并修复 Windows ZIP、自升级状态与运行时生命周期问题。
      - `feat(webui)` / `deps` / `docs`：配置编辑器迁移到 CodeMirror，补齐中英文界面、日志显示、可见性轮询、更新偏好与后端账户隔离；Next.js 升级到 16.3.5，`rustls` 升级到已修复版本，并清除当前 npm / pnpm 与 RustSec 已知漏洞；同步恢复上游 v1.5.x 历史发布记录和当前发布契约。

      **配置与升级说明**

      - 根 crate 版本升级为 `0.2.0`，release tag 为 `v0.2.0`；`oxidns-next-ripset` 升级为 `0.1.3`。已随上游同步升版的 `oxidns-next-proto 0.1.5` 与 `oxidns-next-zoneparser 0.1.2` 保持不变。
      - v0.1.1 的常规 YAML 配置通常可以直接升级；替换二进制前建议运行 `oxidns-next check -c <配置文件>`。本次新增字段均为可选，但用户定义的插件 tag 现在必须是安全 ASCII 路径段，且 `qs.exec.*`、`qs.match.*`、`qs.cron.*` 是保留前缀。
      - matcher 运行时管理 API 已由 `/enable`、`/disable` 改为 `/mode`，使用 `normal`、`always_false` 或 `always_true`；依赖旧接口的自动化需要同步迁移。
      - 查询趋势 API 的新增 bucket 和边界字段向后兼容，原有 `minute` / `hour` 客户端仍可使用。WebUI 只能展示数据库仍保留的数据；完整年度历史需要提前把 `retention_days` 设置为至少 `366`。
      - 使用 RouterOS 插件的部署应先在测试环境核对 comment ownership、TLS 和清理策略；升级后请验证 DNS 查询、管理 API、查询记录和运行时控制。
  </ReleaseCard>
</div>

## 2026-07

<div className="release-stack">
  <ReleaseCard version="v0.1.1" badge="Patch Release" date="2026-07-21">
      **版本定位**

      - Patch Release。v0.1.1 重点修复 WebUI 深色模式下图表提示框不可读，以及切换图表或快速悬停时动画反复追赶、倒放和抖动的问题。
      - 本版本同时让插件拓扑与 Sequence 画布正确跟随主题，完善默认 PostgreSQL / Redis 部署栈，并新增持续依赖安全审计。

      **主要变更**

      - `fix(webui)`：查询统计 Tooltip 显式使用当前主题的 popover 前景、背景和边框色，覆盖 Recharts 的黑色项目默认值，恢复深色模式可读性。
      - `fix(webui)`：关闭查询统计 Tooltip、Bar、Pie 和 Line 的过渡动画，避免切换标签、时间范围、刷新数据或快速前后移动鼠标时动画重复播放、反向追赶或闪动。
      - `fix(webui)`：插件拓扑和 Sequence 编辑画布把 `next-themes` 的实际主题传给 React Flow，使节点、控件和画布在深浅色模式下保持一致。
      - `config` / `docker`：仓库根配置默认使用 PostgreSQL 保存查询历史，并启用 Redis DNS 二级缓存和查询 API 缓存；根 Compose 新增 PostgreSQL 17、Redis 7.4、健康检查、PostgreSQL 持久卷、内部后端网络和 `.env.example` 密码模板。
      - `deps` / `security`：升级 Next.js、Docusaurus 及受公告影响的依赖解析；新增 Security Audit workflow，在相关 push / pull request、每周计划任务和手动触发时执行完整 WebUI / 文档依赖审计与 RustSec。
      - `refactor(tls)`：TLS PEM 证书与私钥加载改为直接使用 `rustls-pki-types`，移除不再维护的 `rustls-pemfile` 直接依赖；证书、私钥路径和 PEM 配置格式保持不变。

      **配置与升级说明**

      - 根 crate 版本升级为 `0.1.1`，release tag 使用 `v0.1.1`；本周期没有 `crates/` 子 crate 代码变更，无需同步升版。
      - 现有运行配置可以直接升级，未新增必填 schema 字段，TLS 证书与私钥配置也无需迁移。
      - 仓库根 `config.yaml` 的部署默认值已改变：现在需要 `OXIDNS_NEXT_QUERY_DATABASE_URL` 与 `OXIDNS_NEXT_REDIS_URL`。使用根 Compose 时，应先复制 `.env.example` 为 `.env`，设置 PostgreSQL / Redis 密码，再启动完整栈。
      - SQLite、MySQL 和自定义单容器部署仍受支持；请保留自己的配置，不要直接用新的根配置覆盖。查询历史只持久化到 SQL 数据库，Redis 仍是可丢弃缓存。
      - RustSec 对 `RUSTSEC-2023-0071` 保留一项经过评估的例外：当前 OIDC / MySQL 依赖路径只执行 RSA 公钥操作，该公告影响私钥计时，且暂无已修复版本。
  </ReleaseCard>

  <ReleaseCard version="v0.1.0" badge="OxiDNS Next 首发" date="2026-07-19">
      **版本定位**

      - OxiDNS Next 首个公开版本，建立独立品牌、发布渠道与管理控制台。本次重新发布的 `v0.1.0` 重点修复大型查询日志数据库下 WebUI 长时间加载或 HTTP 504，并新增 PostgreSQL、MySQL 与 Redis 支持。

      **主要变更**

      - `feat(query_recorder)`：查询日志支持 SQLite、PostgreSQL 和 MySQL。SQLite 仍是零依赖默认选项；生产环境首选 PostgreSQL，MySQL 同样受支持。
      - `fix(query_recorder)`：修复记录与统计请求竞争、已取消 SQLite 读取仍长时间占用资源、以及列表读取不必要的完整记录等问题。列表改为轻量摘要，读取具有超时与取消边界，WebUI 也不再并行发起争用同一读取池的首屏请求。
      - `feat(query_recorder)`：查询历史列表直接显示 IP、CNAME 等实际应答，详情保留完整 DNS 响应；执行路径改为可自然页面滚动的紧凑静态流程，不再拦截滚轮或触摸缩放。
      - `feat(cache)`：新增可选 Redis 共享缓存。DNS `cache` 可使用 Redis 作为二级缓存，`query_recorder` 可缓存记录列表、详情与统计 API 结果。
      - `fix(cache)`：Redis 采用 fail-open 策略。未配置、超时、队列满或暂时不可用时，DNS 回退到进程内缓存与上游，查询日志 API 回退到 SQL 数据库；Redis 不保存唯一数据。
      - `feat(auth)` / `feat(webui)`：提供本地登录、OIDC、通行密钥和 TOTP 多重账户安全选项；查询日志独立展示，仪表盘与插件中心合并。
      - `ci`：新增独立远程存储验证，使用 PostgreSQL 17、MySQL 8.4 和 Redis 7.4 服务容器覆盖数据库与缓存组合。

      **配置与升级说明**

      - 根 crate 版本保持 `0.1.0`，重新发布标签为 `v0.1.0`。
      - 在 `query_recorder.args.database.type` 中选择 `sqlite`、`postgres` 或 `mysql`。PostgreSQL / MySQL 通过 `url`、`max_connections`、`connect_timeout_ms`、`acquire_timeout_ms` 和 `query_timeout_ms` 配置；凭据应使用 `${VAR}` 环境变量提供。
      - 先在顶层 `storage.redis` 配置 Redis URL 与键前缀，再在 DNS `cache.args.redis` 或 `query_recorder.args.api_cache` 中显式启用相应缓存。Redis 是可丢弃缓存，不是查询日志持久化数据源。
      - 此次是对原 `v0.1.0` 的重新发布。已部署旧版的用户应重新下载二进制，或重新拉取 `v0.1.0` / `latest` 镜像。
      - 旧 SQLite 配置可继续使用，遗留的 `query_recorder.args.path` 写法仍兼容。不提供 SQLite 查询日志向 PostgreSQL / MySQL 的迁移工具；切换时请先确认旧日志已不再需要，然后直接使用新库。
  </ReleaseCard>
</div>

## 上游 OxiDNS 历史版本

## 2026-08

<div className="release-stack">
   <ReleaseCard version="v1.5.2" badge="Patch Release" date="2026-08-18">
       **版本定位**

       - Patch Release。v1.5.2 聚焦可信客户端 IP 还原、双栈优选探针隔离、大规则集加载效率与运行时生命周期安全，并补齐 sequence mark 集合操作和发布链路可靠性。
       - v1.5.1 YAML 配置可以直接升级；新增 `client_ip_from_ecs`、`dual_selector.probe_executor` 与 `set_mark` 均为可选能力，现有策略不会因升级自动改变。

       **主要变更**

       - `feat(client_ip_from_ecs)`：新增执行器，可在可信转发端提交 ECS 时把请求局部客户端 IP 替换为 ECS 地址，供后续 client-IP matcher、记录器与策略使用。缺省或空白名单只信任 IPv4/IPv6 loopback，并且仅接受 IPv4 `/32` 或 IPv6 `/128` 完整主机前缀；该插件加入 standard 与 full bundle，minimal 不包含。
       - `feat/fix(dual_selector)`：`prefer_ipv4` / `prefer_ipv6` 新增可选 `probe_executor`，可让 preferred QTYPE 探针走专用 `forward` 或 `sequence`；未配置时继续使用原有 downstream continuation。原始查询与探针使用隔离的子上下文，所有返回路径都会等待或取消两侧任务，插件销毁时同步停止清理任务；缺失引用、类型错误、自引用和循环依赖在启动时拒绝。
       - `feat(sequence)`：`mark` 支持以空格或逗号一次追加多个 `u32` 值；新增 `set_mark` 完整替换当前 mark 集合。重复值自动去重，缺参、负数、非数字与溢出在 sequence 初始化时失败，依赖图、执行路径与 WebUI 编辑器同步识别新语法。
       - `perf/fix(loaders)`：统一 matcher、hosts、redirect、provider、RouterOS 持久化数据与 zone records 的流式文本读取和预分配路径，避免为大文件保留完整文本或中间规则集合；zone parser 新增 visitor API。多轮编译使用指纹校验同一输入，只在完整候选构建成功后发布快照，并把大规模编译移出 async runtime。
       - `fix(runtime/providers)`：provider reload 在调用方取消后仍保持串行 ownership，runtime teardown 会等待在途 reload 与后台构建完成，避免旧快照编译跨越 reload/destroy 边界；AdGuard/V2Ray 等 replay 编译同步补齐来源定位、注释处理与失败回滚覆盖。
       - `fix(download/upgrade)`：共享 HTTP 下载使用具备 Drop 清理的临时文件，超时、取消或失败会移除未完成文件，成功后才原子替换目标；同时修正 Windows ZIP 升级路径类型处理。
       - `deps/ci/release`：切换到包含无损 Tokio response channel 修复的 `oxidns-mikrotik-rs 0.8.1`，移除临时 Git patch 与 crates.io `--no-verify`；升级 `hotpath`、`base64` 等依赖，隔离跨 target 构建缓存，并让 tag workflow 按依赖顺序发布发生版本变化的 workspace support crates。
       - `docs/benchmarks/telegram`：重组中英文安装、配置、CLI、API 与插件参考，新增可复现的多实现 benchmark 场景与结果；Telegram 公告改为保留标题、列表、强调、行内代码和链接的兼容 HTML，并加入长度截断测试。

       **配置与升级说明**

       - 根 crate 版本号升级为 `1.5.2`；`oxidns-proto` 升级为 `0.1.5`，`oxidns-zoneparser` 升级为 `0.1.2`；release tag 应使用 `v1.5.2`。发布流程会先发布新的 support-crate 版本，再发布根 crate。
       - v1.5.1 配置可以直接升级，本次没有重命名或删除配置字段，也没有改变现有插件的默认策略。替换二进制前仍建议运行 `oxidns check -c <配置文件>`。
       - `client_ip_from_ecs` 会改变后续插件观察到的 request-local client IP。请把它放在相关 matcher 与记录器之前，并只把受控反向代理或本机转发器加入 `args`；不要信任可被终端直接访问的来源。转发端必须发送 `/32` 或 `/128` ECS，网络前缀会被忽略。
       - `dual_selector.probe_executor` 未配置时保持 v1.5.1 行为。配置后，探针的上下文 mark、响应和临时状态不会回写原请求，但已经执行的外部副作用无法回滚；专用探针链应优先使用无副作用的解析执行器。
       - 现有单值 `mark` 语法保持有效；只有显式使用 `set_mark` 才会清空原集合。大规则文件若在同一次多轮编译期间发生变化，新候选会被拒绝并继续保留旧快照，运维自动化应在文件完整落盘后再触发 reload。
   </ReleaseCard>
</div>

## 2026-07

<div className="release-stack">
   <ReleaseCard version="v1.5.1" badge="Patch Release" date="2026-07-22">
       **版本定位**

       - Patch Release。v1.5.1 聚焦 matcher 运行时控制、升级运维与 WebUI 质量：将 matcher 的临时启停扩展为三态基础结果控制，补齐强制升级与升级后清理入口，并集中修复国际化、轮询、日志和插件卡片展示。
       - 现有 YAML 配置可以直接升级，但 matcher 运行时管理 API 存在不兼容变更；使用该 API 的客户端必须在升级前迁移。

       **主要变更**

       - `feat/fix(matcher)`：运行时控制改为 `normal`、`always_false`、`always_true` 三态。两个固定模式跳过 matcher 内部逻辑并固定其基础布尔值，每个 `$tag` 或 `!$tag` 引用随后仍独立应用外层取反，因此正向与取反结果始终相反；`sequence`、`any_match` 与 query recorder 同步记录固定模式和最终匹配结果，并增加共享控制器的回归覆盖。
       - `feat(upgrade)`：管理 API 与 WebUI 支持 `force`，可在当前版本已是最新时重新安装；新增 `cleanup` 选项控制成功升级后是否删除下载缓存和备份，WebUI 会持久化两项偏好并生成对应 CLI 命令。清理流程会先释放升级锁，失败时记录告警而不改变已成功的升级结果。
       - `fix(webui/i18n)`：补齐 RouterOS、插件定义、指标、配置历史和控制台组件的中英文文本与本地化日期格式；新增覆盖审计，防止英文界面缺少翻译或回退到中文。
       - `fix(webui/runtime)`：按页面可见性调度运行时轮询，同时保留后台指标采集；不同后端连接的响应、指标基线和升级检查缓存相互隔离，matcher 状态只在首次加载或显式刷新时获取，并在长时间采样中断后重置 QPS 基线。
       - `feat(webui/logs)`：日志查看器支持可持久化的时间戳格式、可选耗时显示、自适应时间单位和 target 路径压缩；插件配置与指标卡片统一为自适应网格，改进 RouterOS 写入结果、时间戳指标和系统内存展示。
       - `perf(build)`：release profile 改用体积优先优化、fat LTO、单 codegen unit 与符号剥离；Tokio、QUIC 和 TLS 依赖仅启用所需 feature，并在 minimal/standard release 流程中尝试使用 UPX 压缩，压缩失败不会阻断发布；crates.io 源码包排除仅供开发的 benchmark、站点文档和 WebUI 源码，避免触及 registry 包体限制。
       - `deps/ci/release`：升级 `wincode`、`syn` 及一组 Rust/GitHub Actions 依赖；RouterOS 暂时通过 Git patch 使用 unbounded response channel 修复突发流量下的协议事件丢失，该 patch 不单独发布，crates.io 发布在上游修复前暂用 `--no-verify`；GitHub Release 与 Telegram 公告统一读取经过版本标题校验的发布说明，公告发送到指定 topic 并自动置顶。

       **配置与升级说明**

       - 根 crate 版本号升级为 `1.5.1`；可发布的 workspace crate 版本保持不变，release tag 应使用 `v1.5.1`。RouterOS patch 不作为独立 crate 发布。
       - v1.5.0 YAML 配置可以直接升级，本次没有新增、重命名或改变默认值的配置字段。替换二进制前仍建议运行 `oxidns check -c <配置文件>`。
       - **Matcher API 迁移**：`POST /api/plugins/<matcher_tag>/enable` 与 `/disable` 已移除，改用 `POST /api/plugins/<matcher_tag>/mode` 并提交 `{ "mode": "normal|always_false|always_true" }`；`GET /status` 的 `enabled` 字段改为 `mode`。未迁移的自动化与第三方控制端会收到 404 或解析失败。
       - matcher 固定模式只保存在当前运行时；应用 reload 或进程重启后恢复为 `normal`。模式由 matcher tag 共享，但引用语义由各引用位置决定：`always_false` 下 `$tag` 不命中而 `!$tag` 命中，`always_true` 下结果相反；如需固定整个 `any_match` 组合，应控制组合 matcher 自身。
       - WebUI 默认在成功升级后清理下载缓存与备份；需要保留本地回滚文件时应关闭“升级后清理”。`force` 会重新安装当前版本，应仅用于修复损坏安装或重新部署相同版本，并在执行前确认目标 bundle 与平台。
       - minimal/standard 产物可能经过 UPX 压缩；依赖二进制扫描、白名单或完整性基线的环境应使用 release asset digest 重新校验，并在生产替换前完成启动与升级回滚演练。
   </ReleaseCard>

   <ReleaseCard version="v1.5.0" badge="Minor Release" date="2026-07-19">
       **版本定位**

       - Minor Release。v1.5.0 以 RouterOS 策略同步和运行时运维能力为主线：新增 `ros_route` 静态策略路由插件，完整重构 `ros_address_list`，并为 matcher/provider 增加管理 API 与 WebUI 运行时控制。
       - 同时新增可配置 `response` 执行器、时区感知的 `time` matcher，完善 query recorder 空间回收、DNS CNAME 响应裁决、WebUI 高级配置、OpenWrt/ARMv7/Docker 发布链路。

       **主要变更**

       - `feat(routeros)`：新增 `ros_route`，将 DNS A/AAAA 观察结果同步为指定 RouterOS routing table 的逐 IP 静态路由，支持 IPv4/IPv6 网关、distance、TTL lease、persistent IP/CIDR、可选 conntrack 延迟删除、启动恢复和受限关闭清理。
       - `refactor(routeros)`：`ros_address_list` 与 `ros_route` 共用 TLS/API-SSL transport、有限并行批处理、按 key 合并的有界队列、租约、reconcile、重试和生命周期原语。RouterOS 不可达不再阻塞 DNS 启动；同步模式受 `wait_timeout` 限制且失败不改变 DNS 应答；删除前重新确认内部 ID、目标 key 与 ownership comment，避免误删外部条目。
       - `feat(runtime_control)`：所有 matcher 支持运行时状态查询、启用和禁用；provider 支持串行化 reload，重复并发请求返回冲突。管理 API、日志和 WebUI 插件详情面板同步接入，并在不启用 API feature 的构建中保持能力关闭。
       - `feat(response/matcher)`：新增 `response` 执行器，可按 zone record 模板生成 Answer/Authority/Additional、RCODE 与响应 flags，并支持 `{qname}`/`{qclass}` 占位符；`time` matcher 支持 IANA 时区、多个时间段、跨午夜窗口、weekday 和 monthday 组合。
       - `refactor(query_recorder)`：统一协调同一 SQLite 数据库的读、写与维护操作；历史清理和定期 retention 按批删除、截断 WAL、迁移旧库到 incremental auto-vacuum 并实际回收磁盘空间，同时保持内存 tail 与数据库结果一致。
       - `fix(dns/forward/cache)`：新增共享 query-aware 响应分类器；并发上游正确区分完整正响应、确定负响应和 incomplete alias，裸 CNAME 不再错误胜出、参与负共识或写入地址缓存；cache dump/load、lazy refresh、TTL 与过期年龄处理同步加固，并减少重复 CNAME 扫描。
       - `feat(webui)`：配置表单增加可折叠高级字段并保留显式默认值、`false`、`0` 与空对象；YAML 编辑器由 Monaco 迁移到本地 CodeMirror，补齐校验、补全和数组/时间段序列化；仪表盘增强 DNS 流量、进程内存与指标不可用状态展示。
       - `feat(release/operations)`：新增 ARMv7 release target；安装脚本可在 OpenWrt 自动安装 `luci-app-oxidns` 及中文包；Docker 运行时切换为 Alpine 构建阶段 + BusyBox musl 镜像；升级模块拆分发布发现、摘要校验、归档与二进制/WebUI 安装，并修正 full/slim target 选择。
       - `fix(config/api/health)`：插件 tag 拒绝路径不安全字符和保留 quick-setup namespace，插件 API 路由统一 URL 编码；health endpoint 在插件初始化完成后再返回健康状态；新增 `response`/RouterOS 配置、feature 与 build-info 能力同步。
       - `deps/ci/docs`：升级依赖与 GitHub Actions 配置，`oxidns-proto` 修复 nightly Clippy 兼容性；整理 server、plugin、infra、upgrade 与 provider/V2Ray 模块边界，并补齐双语 API、插件、OpenWrt、安装和运维文档。

       **配置与升级说明**

       - 根 crate 版本号升级为 `1.5.0`；`oxidns-proto` 升级为 `0.1.4`；release tag 应使用 `v1.5.0`。
       - 大多数 v1.4.0 配置可以直接升级；新增能力均为可选。升级前仍应运行 `oxidns check`。不安全的插件 tag 与保留 quick-setup tag 现在会被拒绝，命中时必须重命名并同步所有引用。
       - **RouterOS ownership 迁移**：`ros_address_list.comment_prefix` 默认值从 `fdns` 改为 `oxi`。若要继续识别、刷新或清理由旧版本创建的条目，请在原插件配置中显式设置 `comment_prefix: fdns`；改用 `oxi` 前请自行处理旧 namespace 条目。
       - `ros_address_list` 新增可选 `tls`、`wait_timeout`、`queue_capacity`；原有 address-list、persistent 与 TTL 配置仍有效。`cleanup_on_shutdown` 仍默认为 `true`，应用 reload 会先关闭旧实例；要求策略连续性的部署应评估设为 `false`，并避免两个进程同时使用相同 tag、comment prefix 和目标列表。
       - `ros_route` 是新插件，要求预先在 RouterOS 创建 routing table/rule 并配置至少一个网关。`fixed_ttl: 0` 会创建不会自然过期的动态路由，且动态租约没有数量上限；启用前必须评估 RouterOS 路由表与 OxiDNS 内存容量。自定义构建可分别使用 `plugin-ros-address-list` / `plugin-ros-route`，`plugin-mikrotik` 继续作为两者的聚合 feature。
       - query recorder 旧数据库会在首次 retention cleanup 或手动清空历史时迁移到 incremental auto-vacuum；大库首次迁移/回收可能产生明显磁盘 I/O，建议避开业务高峰并预留空间。
       - `forward.concurrent: 1` 仍不会隐式重试未启动上游；不完整 CNAME 响应在没有更好结果时仍可返回，但不会作为地址答案缓存。旧 cache dump 中的 CNAME-only 地址项会在加载或命中校验时丢弃。
       - 容器镜像改为 musl/BusyBox 运行时并保留 CA 与时区数据；使用容器、OpenWrt 或 ARMv7 的部署，请先在测试环境验证启动参数、挂载路径、时区和升级/回滚流程。
   </ReleaseCard>
</div>

## 历史归档

版本详情按月份归档，避免当前页面随时间无限增长：

- [2026 年 6 月](releases/2026-06.md)
- [2026 年 5 月](releases/2026-05.md)
- [2026 年 4 月](releases/2026-04.md)
- [2026 年 3 月](releases/2026-03.md)

升级前先阅读目标版本及其间所有版本的“配置与升级说明”。历史记录描述当时行为；当前参数以对应 release tag 的代码和文档为准。
