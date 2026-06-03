# 插件 API 接入与 metrics 划分 Roadmap

最后更新：2026-06-02

本文档梳理 OxiDNS 插件在 **HTTP 管理 API** 与 **Prometheus metrics** 两个观测面上的现状与待办，给后端补接口、给 WebUI 接 API 时提供单一参考。

适用读者：

- 计划给某个插件加 API / metrics 的 Rust 开发者
- 在 WebUI 里给某个插件接 API、做 detail 面板的前端开发者

## 划分原则

API 与 metrics 不重复实现，按数据形态决定归属：

| 数据形态 | 归属 | 例子 |
|---|---|---|
| 计数器、直方图、单值 gauge、低基数标签的汇总值 | **metrics** | QPS、命中率、延迟分布、累计错误数、容量占用、健康标志 |
| 个体可识别的运行时记录 | **API** | 某条规则、某个上游、某个客户端桶、某条任务、某条事件 |
| 配置 / 文本类信息 | **API** | 最近错误消息、TLS 证书、当前路径标识、文件来源 |
| 命令型操作 | **API** | reload、trigger、resync、probe、flush、clear |
| 高基数维度（per-client、per-key、per-domain） | **API**（top-N 枚举） | 限流热点客户端、cache 热 key、PTR 反查 |

简而言之：

- 能写成 `metric{label=...} value` 的，进 metrics
- 必须列名字、列详情、列错误文本、必须点查或下发动作的，进 API

## A. 后端已有 API / WebUI 未对接

无后端工作量，纯前端接入。

| ID | 端点 | 类型 | 说明 |
|---|---|---|---|
| A1 | `POST /api/plugins/<tag>/reload` | 动作 | 由 registry 自动给每个 provider 注册，覆盖 `domain_set` / `dynamic_domain_set` / `geosite` / `adguard_rule` / `ip_set` / `geoip`。WebUI 只有全局 `requestReload()`，缺单 provider 触发 |
| A2 | `GET /api/plugins/<reverse_lookup_tag>?ip=<addr>` | 个体查询 | 从反查缓存返回 FQDN，未命中返回空 |
| A3 | `GET /api/plugins/<query_recorder_tag>/stream` | SSE 事件流 | 已在 `components/plugins/kinds/query-recorder.tsx` 内联使用，没走 `lib/oxidns-api.ts`，建议抽到 lib 与 `streamLogs` 同款 |

## B. 后端新增 API 提案

按"个体 / 文本 / 动作"原则保留，纯计数器一律改为 metrics。

### B1. Provider 通用 `/test`（最高 ROI）

```text
GET /api/plugins/<tag>/test?name=<qname>    # domain_set / dynamic_domain_set / geosite / adguard_rule
GET /api/plugins/<tag>/test?ip=<addr>       # ip_set / geoip
→ { matched: bool, rule?: "domain:example.com", source?: "files[0]" }
```

回答"我这条规则到底能不能命中 X"，一次开发覆盖 6 个 provider。后端在 `provider/mod.rs` 加一个统一 trait 默认实现，前端 detail 加一个输入框。

### B2. `domain_set` / `ip_set` 规则浏览

```text
GET /api/plugins/<tag>/rules?cursor=0&limit=200&q=<filter>
```

复用 `dynamic_domain_set` 的 `/rules` 接口形态与前端组件。

**不在 API 里**：总数 / 各 kind 数量，全部走 metrics（见 C 节）。

### B3. `forward` 上游身份与动作

```text
GET  /api/plugins/<tag>/upstreams
→ [{ idx, addr, proto, healthy: bool, edns0_supported: bool|null,
     last_error_text: string|null, last_error_at: ts|null }]
POST /api/plugins/<tag>/upstreams/<idx>/probe   # 强制健康探测
POST /api/plugins/<tag>/pool/flush              # 强制关闭所有空闲连接
```

**API 提供**：上游身份、二值健康状态、最近错误文本、动作。

**不在 API 里**：连接池数量、空闲 / 在用比例、RTT 直方图（走 metrics）。

### B4. `cron` 任务管理

```text
GET  /api/plugins/<tag>/jobs
→ [{ id, schedule, next_run_at, last_run_at, last_status,
     last_error_text: string|null, paused: bool }]
POST /api/plugins/<tag>/jobs/<id>/run
POST /api/plugins/<tag>/jobs/<id>/pause
POST /api/plugins/<tag>/jobs/<id>/resume
```

**不在 API 里**：运行次数、失败次数（走 metrics）。

### B5. `ip_selector` 评分缓存

```text
GET    /api/plugins/<tag>/scores?limit=200
DELETE /api/plugins/<tag>/scores
DELETE /api/plugins/<tag>/scores/<ip>
```

**不在 API 里**：总条目数（已有 `ip_selector_cache_entries` metric）。

### B6. `ipset` / `nftset` 内核 set 影子表

```text
GET  /api/plugins/<tag>/entries
POST /api/plugins/<tag>/resync     # 强制从内核重读并推送一遍
```

### B7. `ros_address_list` RouterOS 影子表

```text
GET  /api/plugins/<tag>/entries
POST /api/plugins/<tag>/resync     # 强制与 RouterOS 重新对账
```

### B8. `fallback` 当前路径

```text
GET /api/plugins/<tag>/state
→ { current_path: "primary"|"secondary",
    last_failover_at: ts|null,
    recent_failovers: [{at, from, to, reason}] }
```

**不在 API 里**：失败率、命中数（走 metrics）。建议另外加 `fallback_current_path{tag,path}` 的 0/1 gauge。

### B9. `hosts` 条目枚举

```text
GET /api/plugins/<tag>/entries?q=<filter>
```

**不在 API 里**：total、last_reload_at（都该是 metric，见 C 节）。

### B10. `download` 状态与触发

```text
GET  /api/plugins/<tag>/status
→ { last_outcome: "success"|"failed"|"running",
    last_error_text: string|null,
    last_success_at: ts|null,
    last_attempt_at: ts|null }
POST /api/plugins/<tag>/trigger
```

### B11. `learn_domain` 最近学习事件

```text
GET /api/plugins/<tag>/recent?limit=200
→ [{ qname, qtype, at, source }]
```

环形缓冲。**不在 API 里**：写入总数、写入失败数（走 metrics）。

### B12. `rate_limiter` 热点客户端

```text
GET /api/plugins/<tag>/buckets?top=20
→ [{ client_subnet, tokens, last_refill_at, throttled_1m }]
```

per-client 是高基数维度（IPv4/24、IPv6/48），不能放进 Prometheus 标签。API 是唯一合适的暴露方式。

### B13. `prefer_ipv4` / `prefer_ipv6` 学习缓存

```text
GET    /api/plugins/<tag>/cache              # 已学到的 preferred-only 域名
DELETE /api/plugins/<tag>/cache
DELETE /api/plugins/<tag>/cache/<name>
```

### B14. `script` 错误与重载

```text
GET  /api/plugins/<tag>/status
→ { compile_error: string|null,
    last_panic: string|null,
    last_panic_at: ts|null,
    loaded_at: ts }
POST /api/plugins/<tag>/reload
```

**不在 API 里**：调用计数（走 metrics）。

### B15. `cache` 热 key 枚举

```text
GET /api/plugins/<tag>/top?by=hits&limit=20
```

per-key 是高基数维度。**不在 API 里**：hit_rate、entries、evictions_total、oldest_ms、newest_ms、ecs_distinct_keys（全部 metrics）。

## C. 补 metrics 建议

以下 gauge / counter 不应该放进 API。如果 Rust 端尚未暴露，建议补 metrics。

| 插件 | 建议 metric |
|---|---|
| `domain_set` / `ip_set` | `provider_rules_total{tag,kind}`、`provider_files_loaded{tag}`、`provider_last_reload_timestamp_seconds{tag}` |
| `forward` | `forward_upstream_healthy{tag,upstream}` (0/1)、`forward_upstream_edns0_supported{tag,upstream}` (0/1)、`forward_pool_connections{tag,upstream,state="idle\|in_use"}` |
| `hosts` | `hosts_entries_total{tag}`、`hosts_last_reload_timestamp_seconds{tag}` |
| `fallback` | `fallback_current_path{tag,path}` (0/1)、`fallback_failover_total{tag,direction}` |
| `cron` | `cron_job_runs_total{tag,job,result}`、`cron_job_last_run_timestamp_seconds{tag,job}` |
| `download` | `download_attempt_total{tag,result}`、`download_last_success_timestamp_seconds{tag}` |
| `script` | `script_calls_total{tag,result}`、`script_panics_total{tag}` |
| `learn_domain` | `learn_domain_written_total{tag}`、`learn_domain_dropped_total{tag,reason}`（多数已有，请核实） |

## D. 维持现状（无须 API）

- **全部 19 个 matcher**：无运行时状态，规则数据在引用的 provider 里。
- **简单 executor**：`sequence`、`debug_print`、`sleep`、`drop_resp`、`black_hole`、`query_summary`、`arbitrary`、`redirect`、`ttl`、`ecs_handler`、`forward_edns0opt`、`reload`、`reload_provider`、`upgrade`。
- **4 个 server**（`udp_server` / `tcp_server` / `http_server` / `quic_server`）：TLS 证书过期时间、监听地址、活跃连接数都放进全局 `/api/system` 与 metrics 增强，不开 plugin 级 API。

## E. 推荐落地次序

| 阶段 | 内容 | 投入 | 收益 |
|---|---|---|---|
| 1 | A1 + A2 + A3 | 半天，纯前端 | 立刻填补已有 API 的 WebUI 空白 |
| 2 | B1 provider 通用 `/test` | 1 天，单 trait + 单前端组件 | 一次开发覆盖 6 个 provider，解决最常见的"规则匹配验证"问题 |
| 3 | B2 domain_set / ip_set rules 枚举 + C 节对应 metrics | 1 天 | 解决"配了 files 但看不到加载了什么" |
| 4 | B3 forward 上游身份 + 动作 + healthy / edns0 gauge metric | 1.5 天 | forward 是核心路径，目前最不可观测 |
| 5 | B6 / B7 / B11 / B12 / B13（轻量个体枚举） | 各 0.5 天 | 按运维频率排队 |
| 6 | B4 cron / B14 script / B10 download / B5 ip_selector / B8 fallback / B15 cache top | 各 1 天 | 按实际使用频率推 |

## F. 实施模板

### 后端：给一个新插件加 API 路由

```rust
// 在插件 init() 里
register_plugin_api!(
    self.tag(),
    GET "/entries" => MyListHandler { backend: self.backend.clone() },
    POST "/resync"  => MyResyncHandler { backend: self.backend.clone() },
)?;
```

参考实现：`src/plugin/provider/dynamic_domain_set/api.rs`、`src/plugin/executor/cache/api.rs`。

### 前端：包装一个新端点

```typescript
// lib/oxidns-api.ts
export async function fooBar(tag: string, params: ...): Promise<...> {
  const response = await fetch(apiUrl(`/plugins/${encodeURIComponent(tag)}/...`), {
    method: "...",
    headers: apiHeaders(),
    ...
  });
  return readJsonResponse<...>(response);
}
```

参考实现：`lib/oxidns-api.ts` 里的 `listDynamicDomainRules` / `appendDynamicDomainRules` 等。

### 前端：给插件加 detail tab

参考实现：`components/plugins/kinds/dynamic-domain-set.tsx`（规则管理 tab）、`components/plugins/kinds/query-recorder.tsx`（多 tab 历史 + SSE 流）。注册在 `components/plugins/registry.ts`。
