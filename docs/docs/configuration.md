---
title: 配置总览
sidebar_position: 2
---

OxiDNS Next 使用 YAML 描述运行参数、管理接口、共享网络设置和插件执行链。本页用于选择正确的配置主题；完整可运行示例仍以仓库根目录的 `config.yaml` 为准。

OxiDNS Next 的配置文件是 YAML。日常修改配置时，可以先把它理解为七个顶层部分：

```yaml
runtime:
  worker_threads: 4

api:
  http: "127.0.0.1:9088"

log:
  level: info
  file: ./oxidns-next.log

storage:
  redis:
    url: ${OXIDNS_NEXT_REDIS_URL}
    key_prefix: oxidns-next

network:
  outbound:
    default: direct
    profiles:
      direct:
        resolver: system
        proxy: none

include: []

plugins:
  - tag: seq_main
    type: sequence
    args:
      - exec: "forward 1.1.1.1"
```

其中：

- `runtime`
  - 运行时参数。
- `api`
  - 管理 API。
- `log`
  - 日志输出。
- `storage`
  - 可选共享存储连接；当前用于给 DNS cache 和查询日志 API 提供 Redis 缓存。
- `network`
  - 共享网络出站配置，例如 HTTP 下载、升级检查和 webhook 请求使用的解析器与代理。
- `include`
  - 从其他配置文件载入插件定义。
- `plugins`
  - 所有插件实例定义。OxiDNS Next 通过插件组合完成完整 DNS 流程。

修改完成后，建议先校验再启动：

```bash
oxidns-next check -c config.yaml
```

如果部署把配置与运行数据分开，校验时必须提供真实工作目录。例如 Debian 默认布局使用：

```bash
oxidns-next check -c /etc/oxidns-next/config.yaml -d /var/lib/oxidns-next
```

## 章节导航

| 主题 | 包含内容 |
| --- | --- |
| `${VAR}` | 使用进程环境变量 `VAR` 的值；未定义时报错 |
| `${VAR:-default}` | `VAR` 未定义或为空字符串时使用 `default` |
| `${env:VAR}` | 显式读取进程环境变量 `VAR`，可用于变量名与运行期占位符冲突的场景 |
| `${env:VAR:-default}` | 显式读取进程环境变量 `VAR`，未定义或为空字符串时使用 `default` |
| `$${...}` | 输出字面量 `${...}` |

`script`、`http_request` 等执行器使用的运行期占位符会被保留到请求执行阶段再渲染，例如 `${qname}`、`${client_ip}`、`${resp_ip}` 不会在配置加载时当作进程环境变量处理。如果确实需要读取同名环境变量，请使用显式写法，例如 `${env:qname}`。

未定义变量会立即报错，错误中包含变量名和发生位置的 YAML 路径（例如 `plugins[0].args.password`），避免空密码、空证书路径等问题静默通过。

示例：

```yaml
api:
  http:
    listen: ${API_LISTEN:-0.0.0.0:8080}
    ssl:
      cert: ${API_TLS_CERT}
      key: ${API_TLS_KEY}
    auth:
      type: accounts
      database: ${OXIDNS_NEXT_AUTH_DB:-./data/oxidns-next-auth.db}
      bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
```

因为替换发生在 YAML 解析之后，环境变量值可以包含任意字符——`*`、`&`、`:`、`#`、`'`、`"`、`\`、换行甚至二进制字节——都不会破坏配置文件的语法。不需要为含特殊字符的值手动加引号。当整段标量恰好等于一个占位符时（例如 `timeout: ${CACHE_TTL}`），展开结果会按 YAML 1.2 标量规则做一次类型恢复，所以数字、布尔、`null` 形态的环境变量仍能匹配数字 / 布尔 / 空类型字段；其他位置一律按字符串处理。`include` 路径同样支持占位符，例如：

```yaml
include:
  - ${OXIDNS_NEXT_CONF_DIR}/plugins/common.yaml
```

## 顶层字段

### `include`

```yaml
# []string, 从其他配置文件载入 plugins 插件设置。
include:
  - ./plugins/common.yaml
  - ./plugins/server.yaml
```

字段说明：

- `include`
  - 只载入被包含文件中的 `plugins`，不会合并被包含文件的 `runtime`、`api` 或 `log`。
  - 插件合并顺序为：先按数组顺序递归载入 `include`，再追加当前文件的 `plugins`。
  - 相对路径以声明该 `include` 的配置文件所在目录为基准。
  - 最多递归 8 层。
  - 合并后的所有插件 `tag` 仍必须全局唯一。

### `runtime`

```yaml
runtime:
  worker_threads: 4
```

字段说明：

- `worker_threads`
  - 含义：Tokio 多线程运行时的 worker 数。
  - 默认：未配置时自动取系统可用并行度。
  - 限制：不能为 `0`。

### `log`

```yaml
log:
  level: info
  file: ./oxidns-next.log
  query_file: ./data/query-events.log
  rotation:
    type: daily
    max_files: 7
```

字段说明：

- `level`
  - 可选值：`off` `trace` `debug` `info` `warn` `error`
  - 默认：`info`
- `file`
  - 含义：可选日志文件路径。
  - 不配置时仅输出到标准输出。
  - 配置后，OxiDNS Next 会同时输出到标准输出和日志文件。
  - 日志文件内容为 UTF-8 纯文本格式，不写入终端 ANSI 颜色控制码。
- `query_file`
  - 含义：可选的 DNS 查询事件日志文件；仅接收 `debug_print` 与 `query_summary` 等查询诊断事件。
  - 不配置时不写入查询事件文本文件；结构化、可检索的查询历史仍由 `query_recorder` 独立保存在其配置的 SQLite、PostgreSQL 或 MySQL 数据库中。
  - 如需使用 `debug_print` 或 `query_summary` 的文本输出，应配置此字段；省略时这些查询诊断事件不会进入系统日志。可检索历史不受影响。
  - 查询事件可能包含客户端地址和域名，必须限制文件访问权限并按隐私策略设置保留期。
- `rotation`
  - 含义：日志文件轮转策略。
  - 默认：`never`

`rotation` 支持以下配置：

- `type: never`
  - 不轮转，始终写入同一个文件。
- `type: minutely`
  - 按分钟轮转。
- `type: hourly`
  - 按小时轮转。
- `type: daily`
  - 按天轮转。
- `type: weekly`
  - 按周轮转。
  - 可选配置 `max_files`，表示最多保留多少个历史文件；`0` 表示不自动删除。

### `storage`

`storage` 定义可供多个插件复用的外部存储连接。当前支持可选 Redis：

```yaml
storage:
  redis:
    url: "${OXIDNS_NEXT_REDIS_URL}"
    key_prefix: "oxidns-next"
    connect_timeout_ms: 1000
```

字段说明：

- `storage.redis.url`
  - 类型：`string`；必填：是。
  - 含义：Redis 连接 URL，例如 `redis://redis:6379/0`。含账号或密码时应通过环境变量提供，不要直接提交到配置文件。
- `storage.redis.key_prefix`
  - 类型：`string`；必填：否；默认：`oxidns-next`。
  - 含义：所有 OxiDNS Next Redis key 的命名空间前缀。同一 Redis 部署运行多个实例时应使用不同前缀。
- `storage.redis.connect_timeout_ms`
  - 类型：`integer`；必填：否；默认：`1000`。
  - 含义：建立 Redis 连接的超时毫秒数。

只配置 `storage.redis` 不会自动启用缓存。DNS `cache` 需配置 `args.redis`，`query_recorder` 需配置 `args.api_cache`。Redis 只保存可丢弃的缓存数据：网络错误、认证失败、超时、队列满、熔断或缓存内容无效时，DNS 会回退到进程内缓存与上游，查询日志 API 会回退到 SQL 数据库。不要把 Redis 当作查询日志的持久化数据源。

如果构建未包含 `storage-redis` feature，却在插件中启用了 Redis，配置校验会给出不支持该能力的错误。完整 PostgreSQL、MySQL 与 Redis Compose 示例见仓库 [`examples/storage`](https://github.com/ciallothu/oxidns-next/tree/main/examples/storage)。

### `network`

`network.outbound` 用于集中管理项目内部 HTTP client 与 upstream 出站策略。未配置时保持兼容行为：HTTP client 使用系统 DNS 解析并直连目标地址，upstream 保持自身配置。

```yaml
network:
  outbound:
    default: direct
    profiles:
      direct:
        resolver: system
        proxy: none
      remote:
        resolver:
          nameservers:
            - addr: "1.1.1.1:53"
            - addr: "tls://dns.google:853"
              dial_addr: 8.8.8.8
            - addr: "https://cloudflare-dns.com/dns-query"
              dial_addr: 1.1.1.1
          ip_version: 4
          timeout: 5s
          proxy: none
        proxy:
          socks5: 127.0.0.1:1080
```

字段说明：

- `outbound.default`
  - 含义：未显式配置 `outbound` 的 HTTP client 和 upstream 默认使用哪个 profile。
  - 默认：无；无默认 profile 时使用系统 DNS + 直连。
  - 限制：如果配置，必须引用 `profiles` 中存在的名称。
  - 注意：默认 profile 的 proxy 会严格应用到 upstream；如果默认 SOCKS5 proxy 遇到 UDP、DoQ 或 DoH3 upstream，启动会失败，因为这些连接模型不支持 profile proxy。
- `outbound.profiles.<name>.resolver`
  - `system`：使用系统 DNS。HTTP client 中该解析是异步执行，不会阻塞运行时工作线程。
  - `nameservers`：使用指定 DNS nameserver 解析目标域名。支持 `udp://`、`tcp://`、`tls://`、`https://`、`doh://`、`h3://`、`quic://`、`doq://`；未写协议时按 UDP 处理。
  - 协议 feature：UDP/TCP 总是可用；DoT 需要 `resolver-dot`，DoH 需要 `resolver-doh`，DoQ 需要 `resolver-doq`，DoH3 需要 `resolver-doh3`。旧的 `upstream-*` feature 仍会启用共享 DNS client 依赖以兼容既有构建脚本，但新配置建议显式启用 `resolver-*`。
  - `ip_version`：可选，`4` 查询 A 记录，`6` 查询 AAAA 记录；未配置时默认 IPv4。
  - `timeout`：可选，resolver 查询超时，默认 `5s`。
  - `proxy`：可选，`none` 表示 nameserver 直连，`profile` 表示 TCP/DoT/DoH nameserver 复用当前 profile 的 SOCKS5。UDP/DoQ/DoH3 nameserver 不支持 SOCKS5。
  - 域名型 nameserver 必须配置 `dial_addr`，`addr` 中的域名用于 SNI/证书校验，`dial_addr` 用于实际连接，避免 resolver 解析自身。
- `outbound.profiles.<name>.proxy`
  - `none` 或 `direct`：直连。
  - `socks5`：通过 SOCKS5 代理连接目标地址，格式与上游 `socks5` 一致。

当前 `download`、`upgrade`、`http_request` 可通过 `args.outbound: remote` 引用 profile。旧字段 `socks5` 继续兼容；当同一个插件同时配置 `outbound` 和 `socks5` 时，`socks5` 会覆盖 profile 中的代理设置，但 resolver 仍来自该 outbound profile。`forward` upstream 未配置 `outbound` 时会使用 `network.outbound.default`；也可通过 `outbound: remote` 显式接入其他 profile。upstream 本地 `dial_addr`、`bootstrap`、`socks5` 优先于 profile 注入值。

### `api`

`api.http` 支持两种写法。

简写：

```yaml
api:
  http: "127.0.0.1:9088"
```

详写：

```yaml
api:
  http:
    listen: "127.0.0.1:9443"
    ssl:
      cert: "/etc/oxidns-next/api.crt"
      key: "/etc/oxidns-next/api.key"
      client_ca: "/etc/oxidns-next/client-ca.crt"
      require_client_cert: true
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      bootstrap_token_env: OXIDNS_NEXT_BOOTSTRAP_TOKEN
      session_ttl_seconds: 43200
      cookie_same_site: lax
      public_url: "https://dns.example.com"
      passkey:
        rp_id: "dns.example.com"
        origins: ["https://dns.example.com"]
    webui:
      root: "/etc/oxidns-next/webui"
      index: "index.html"
```

字段说明：

- `http.listen`
  - API 监听地址，支持 `ip:port`、`[ipv6]:port` 和 `:port`。
  - `:port` 会绑定为双栈 `[::]:port`；仅监听 IPv4 时请显式写 `0.0.0.0:port`。
- `http.ssl.cert`
  - API 证书文件。
- `http.ssl.key`
  - API 私钥文件。
- `http.ssl.client_ca`
  - 可选客户端证书 CA。
- `http.ssl.require_client_cert`
  - 是否要求双向 TLS。
- `http.auth`
  - 新部署使用 `type: accounts`，以 SQLite 保存本地账户、TOTP、通行密钥、OIDC 绑定与会话。
  - `type: basic` 仅保留为上游配置的一次性迁移入口；导入账户库后应删除 YAML 明文密码。
- `http.auth.database`
  - 账户数据库路径，默认 `./data/oxidns-next-auth.db`，相对路径以工作目录为基准。数据库及其备份包含密码哈希、TOTP secret、通行密钥和会话安全信息，必须按敏感凭据保护；Unix 上运行时会把数据库权限收紧为 `0600`。
- `http.auth.bootstrap_token` / `bootstrap_token_env`
  - 非直接 loopback（包括通过本机反向代理）创建首个管理员所需的一次性 token；二者只能配置一个。反向代理和远程引导应使用 `bootstrap_token_env`，完成后移除该环境变量，不要把 token 保留在 YAML 中。
- `http.auth.session_ttl_seconds`
  - 会话有效期，范围 300 到 604800 秒，默认 43200。
- `http.auth.cookie_secure`
  - 可选覆盖 Secure Cookie 自动判断；HTTPS 生产部署通常保持未设置。
- `http.auth.cookie_same_site`
  - 支持 `lax`（默认）、`strict` 与 `none`；跨站点 WebUI 使用 `none` 时必须同时启用 Secure Cookie 和精确 CORS origin。
- `http.auth.public_url`
  - 浏览器可见的绝对 HTTP(S) 地址，用于通行密钥与回调 origin 推导；在反向代理终止 TLS 时，它也是公共认证接口接受的精确可信 origin。应配置为浏览器实际访问 API 的地址，不依赖 `X-Forwarded-*` 请求头。
- `http.auth.passkey`
  - `rp_id` 与 `origins` 可显式配置；未提供时必须能从 `public_url` 推导。
- `http.auth.oidc`
  - 配置 `issuer_url`、`client_id`、client secret、`redirect_url` 和 `allowed_users`。
  - `allowed_users` 将身份提供方 claim 显式映射到已有本地账户；OIDC 不自动创建管理员。
  - client secret 应通过 `client_secret_env` 注入，不要写入 YAML；`client_secret` 仅作为兼容配置保留，二者不能同时设置。
- `http.cors.allowed_origins`
  - 可选的 WebUI/API 跨域白名单。
  - 认证关闭时，未配置的规则会根据 `http.listen` 自动推导：`0.0.0.0` 和 `[::]` 允许任意 origin，具体 IP 允许同一 host 的任意 WebUI 端口。
  - 启用账户认证后不会使用上述宽松推导；同源 WebUI 无需配置 CORS，跨源且携带会话 Cookie 的 WebUI 必须显式列出每个精确 origin。
  - 显式配置时按浏览器 `Origin` 精确匹配。
  - 使用 `"*"` 可允许任意 origin，但不能与浏览器凭据跨域一起使用。
- `http.webui.root`
  - 可选的 WebUI 静态文件目录。启用后 WebUI 挂载在 `/`，管理 API 位于 `/api/*`。
  - 相对路径以 `-d/--working-dir` 为基准；例如 Debian service 默认 `-d /var/lib/oxidns-next`，因此 `root: "./webui"` 表示 `/var/lib/oxidns-next/webui`。
  - WebUI 构建、发布目录和 nginx 独立部署方式见《[WebUI 部署](webui.md)》。
- `http.webui.index`
  - 可选首页文件名，默认 `index.html`。

校验规则：

- `listen` 不能为空。
- `cert` 和 `key` 必须成对出现。
- `require_client_cert: true` 时必须提供 `client_ca`。
- `accounts.database` 不能为空；`bootstrap_token` 与 `bootstrap_token_env` 不能同时配置。
- `session_ttl_seconds` 必须在 300 到 604800 之间。
- `cookie_same_site: none` 不能与 `cookie_secure: false` 组合，且运行时必须能确定 Cookie 为 Secure。
- 启用 OIDC 时必须提供合法的 issuer、client ID、redirect URL、包含 `openid` 的 scopes，以及至少一条 `allowed_users` 映射。
- 启用通行密钥时必须通过 `public_url` 或 `rp_id` + `origins` 提供浏览器作用域。
- `webui.root` 不能为空。
- `webui.index` 配置后不能为空。

### `plugins`

每个插件定义都采用统一结构：

```yaml
- tag: cache_main
  type: cache
  args:
    size: 4096
```

通用规则：

- `tag`
  - 插件实例唯一标识。
  - 不能为空。
  - 在整个配置中必须唯一。
- `type`
  - 插件类型名。
  - 必须与已注册插件工厂一致。
- `args`
  - 插件参数。
  - 不同插件的参数形态不同，可能是对象、字符串、数组或空值。

## 四类插件的职责

### `server`

作用：接收 DNS 请求并把请求送入某个执行器入口。

## 配置所有权

- 本章解释所有插件共享的配置模型，不重复维护各插件的字段表。
- 插件专属参数以[插件参考](plugin-reference/overview.md)为准。
- CLI 参数以[命令行工具](cli.md)为准，HTTP 结构以[管理 API](api.mdx)为准。
- 发布包中的 `config.yaml` 是当前版本的规范可运行示例；升级后应使用新二进制重新执行 `oxidns-next check`。

<span id="include"></span><span id="runtime"></span><span id="log"></span><span id="network"></span><span id="api"></span><span id="plugins"></span>

从旧版书签进入本页时，请使用上表跳转到拆分后的主题页。
