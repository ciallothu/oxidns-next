---
title: 配置总览
sidebar_position: 2
---

## 写在最前

OxiDNS Next 的配置文件是 YAML。日常修改配置时，可以先把它理解为六个顶层部分：

```yaml
runtime:
  worker_threads: 4

api:
  http: "127.0.0.1:9088"

log:
  level: info
  file: ./oxidns-next.log

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

如果配置中使用了相对路径，并且实际工作目录不是配置文件所在目录，可以配合 `-d` 指定工作目录。`-d` 是日志、SQLite、规则文件、`api.http.webui.root` 等所有运行期相对路径的统一基准，不会因为配置文件位于 `/etc/oxidns-next` 而自动改到配置目录：

```bash
oxidns-next check -c /etc/oxidns-next/config.yaml -d /var/lib/oxidns-next
```

Debian 默认布局中，配置文件放在 `/etc/oxidns-next/config.yaml`，运行期相对路径资源放在 `/var/lib/oxidns-next`。

尚未确定插件组合方式时，建议先阅读《[常见策略场景](scenarios.md)》，再回到本页查询字段含义。

## 环境变量替换

OxiDNS Next 在启动、`oxidns-next check`、管理 API 配置校验和保存前校验时，先把 YAML **解析成数据结构**，再在字符串标量内部展开 `${VAR}` 占位符。`config.yaml` 文件本身不会被改写；WebUI 读取和保存配置时看到的仍然是原始占位符。

支持的写法：

| 写法 | 行为 |
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
  - 不配置时不写入查询事件文本文件；结构化、可检索的查询历史仍由 `query_recorder` 独立保存在其 SQLite 数据库中。
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

特点：

- 不负责复杂策略判断。
- 核心配置通常是监听地址、TLS 参数、入口执行器。

### `executor`

作用：执行动作。

典型动作包括：

- 查询上游
- 生成本地响应
- 缓存读写
- TTL 调整
- ECS 处理
- 回退和并发竞争
- 观测与系统联动

### `matcher`

作用：做条件判断，供 `sequence` 规则使用。

典型判断维度包括：

- 查询域名
- 查询类型
- 客户端 IP
- 应答 IP
- 应答码
- 环境变量
- 采样命中
- 限流状态

### `provider`

作用：提供可复用规则集，供 `matcher` 或其它插件引用。

当前主要有：

- `domain_set`
- `ip_set`
- `geoip`
- `geosite`
- `adguard_rule`

## sequence 编排模型

`sequence` 是 OxiDNS Next 的策略中枢。绝大多数非平凡配置都会以它作为总入口。

示例：

```yaml
- tag: seq_main
  type: sequence
  args:
    - matches:
        - "$lan_clients"
        - "qtype A,28"
      exec: "$cache_main"
    - matches: "!has_resp"
      exec: "$forward_main"
    - exec: "accept"
```

每条规则支持两个核心字段：

- `matches`
  - 一个 matcher 表达式或表达式数组。
  - 数组中的所有条件都成立时，本条规则才命中。
- `exec`
  - 命中后执行的动作。

## 引用插件与 quick setup

### 引用已有插件

使用 `$tag` 引用已定义插件：

```yaml
- exec: "$forward_main"
- matches:
    - "$is_internal"
    - "!has_resp"
  exec: "$cache_main"
```

### quick setup

如果 `sequence` 中写的不是 `$tag`，而是 `type + 参数` 形式，OxiDNS Next 会即时构造临时插件。

示例：

```yaml
- exec: "forward 1.1.1.1 8.8.8.8"
- matches: "qname domain:example.com"
  exec: "ttl 300"
```

当前常见 quick setup：

- matcher
  - `_true`
  - `_false`
  - `qname ...`
  - `qtype ...`
  - `qclass ...`
  - `client_ip ...`
  - `resp_ip ...`
  - `ptr_ip ...`
  - `cname ...`
  - `mark ...`
  - `env ...`
  - `random ...`
  - `rate_limiter ...`
  - `rcode ...`
  - `has_resp`
  - `has_wanted_ans`
  - `string_exp ...`
- executor
  - `forward ...`
  - `cache ...`
  - `ttl ...`
  - `prefer_ipv4`
  - `prefer_ipv6`
  - `sleep ...`
  - `debug_print ...`
  - `query_summary ...`
  - `metrics_collector ...`
  - `black_hole ...`
  - `drop_resp`
  - `ecs_handler ...`
  - `forward_edns0opt ...`
  - `ipset ...`
  - `nftset ...`
  - `upgrade ...`
  - `download ...`
  - `reload_provider ...`
  - `reload`

## sequence 内建控制流

除了调用插件，`sequence.args[].exec` 还可以直接写内建控制流：

### `accept`

- 立即结束当前 `sequence`。
- 这是一次明确的提前停止，因此调用方不会继续执行后续规则。
- 不会自动生成响应。
- 典型用法：
  - `cache`、`hosts`、`arbitrary` 等前置 executor 已经写入 response 后，直接收口。
  - 命中某个分支后明确不希望再进入后续 `forward` / 副作用逻辑。

### `return`

- 立即结束当前 `sequence`，把控制权交回调用方。
- 不会自动生成响应。
- 如果当前 `sequence` 是被 `jump` 调用的，调用方会从 `jump` 后一条规则继续执行。
- 如果当前 `sequence` 是顶层入口，它等价于“提前结束当前规则链”。

### `reject [rcode]`

- 立即基于当前 request 构造一个 DNS 响应，并结束当前 `sequence`。
- 默认 `rcode` 为 `REFUSED`，所以 `reject` 等价于拒绝请求。
- 可以显式写十进制数值或英文 RCODE 名称；英文名称大小写不敏感。常见映射与含义见 [DNS 编码速查表](dns-codes.md#rcode-响应码)，例如：
  - `reject 2` => `SERVFAIL`
  - `reject SERVFAIL` / `reject servfail` => `SERVFAIL`
  - `reject 3` => `NXDOMAIN`
  - `reject NXDOMAIN` => `NXDOMAIN`
- `reject` 只支持基础 DNS RCODE `0..15`；扩展 RCODE 需要 EDNS OPT，不会由该内建动作自动生成。
- `reject 0` 只返回普通 `NOERROR` 响应，不会自动附加 SOA。
- 调用方不会继续执行后续规则。
- 典型用法是直接返回指定错误码，例如：

```yaml
- matches: "qtype HTTPS"
  exec: "reject NXDOMAIN"
```

### `mark ...`

- 向 `DnsContext.marks` 写入一个或多个无符号整数 mark。
- 支持写法：
  - `mark 1`
  - `mark 1 2 3`
  - `mark 1,2,3`
- 写入后会继续执行当前 `sequence` 的下一条规则。
- 它本身不会生成响应，也不会终止当前 `sequence`。

### `jump seq_tag`

- 调用另一个 `sequence`，语义上类似“子过程调用”。
- 参数必须是目标 `sequence` 的 tag，且不能写 `$` 前缀。
- 被调用的 `sequence` 如果：
  - 正常执行到尾部，当前 `sequence` 会从 `jump` 的下一条规则继续。
  - 中途执行了 `return`，当前 `sequence` 也会从 `jump` 的下一条规则继续。
  - 中途执行了 `accept`、`reject` 或其它返回 `Stop` 的操作，当前 `sequence` 也会一起停止，不再继续后续规则。

### `goto seq_tag`

- 直接把控制权转交给另一个 `sequence`，语义上类似“单向跳转”。
- 参数必须是目标 `sequence` 的 tag，且不能写 `$` 前缀。
- 当前 `sequence` 在执行 `goto` 后不会恢复：
  - 目标 `sequence` 正常跑到尾部，不回到 `goto` 后面的规则。
  - 目标 `sequence` 执行 `return`，该 `return` 会继续向外层传播，但同样不回到 `goto` 后面的规则。
  - 目标 `sequence` 执行 `accept` / `reject` / 其它 `Stop`，结果也直接向外层传播。
- 适合把请求永久移交给另一个策略分支。

示例：

```yaml
- matches: "$rate_ok"
  exec: "mark 100"
- matches: "!$rate_ok"
  exec: "reject 2"
```

`jump` / `goto` 的区别示例：

```yaml
- tag: child_seq
  type: sequence
  args:
    - exec: "mark 2"
    - exec: "return"

- tag: parent_jump
  type: sequence
  args:
    - exec: "mark 1"
    - exec: "jump child_seq"
    - exec: "mark 3"

- tag: parent_goto
  type: sequence
  args:
    - exec: "mark 1"
    - exec: "goto child_seq"
    - exec: "mark 3"
```

- `parent_jump` 最终会留下 `1,2,3`，因为 `jump` 调用结束后会继续执行下一条。
- `parent_goto` 最终只会留下 `1,2`，因为控制权不会回到 `goto` 之后。

## 通用规则语法

### 域名规则

以下规则会出现在 `qname`、`cname`、`domain_set`、`hosts`、`redirect` 等插件中：

- `full:example.com`
  - 完整匹配。
- `domain:example.com`
  - 后缀匹配。
- `keyword:cdn`
  - 子串匹配。
- `regexp:^api[0-9]+\\.example\\.com$`
  - 正则匹配。
- `example.com`
  - 未写前缀时，`qname`、`cname`、`domain_set` 等通用域名规则通常等价于 `domain:example.com`；`hosts` 和 `redirect` 按 `full:example.com` 精确匹配处理。

### IP 规则

以下规则会出现在 `client_ip`、`resp_ip`、`ptr_ip`、`ip_set` 等插件中：

- 单个 IP：`1.1.1.1`
- 网段：`192.168.0.0/16`
- IPv6 网段：`2400:3200::/32`

### provider 引用

支持在 matcher 或 provider 参数中引用 provider：

- `$tag`
  - 引用已定义且具备对应匹配能力的 provider。
  - 例如域名场景可引用 `domain_set`、`geosite`。
  - 例如 IP 场景可引用 `ip_set`、`geoip`。
- `&/path/to/file`
  - 直接从文件加载规则。

示例：

```yaml
args:
  - "domain:example.com"
  - "$core_domains"
  - "&/etc/oxidns-next/domains.txt"
```
