---
title: WebUI 部署
sidebar_position: 5
---

OxiDNS Next WebUI 是独立构建的前端静态产物，不会编译进 Rust 后端二进制。部署时有两种推荐方式：

- 后端同端口托管：由 OxiDNS Next 管理 HTTP 服务直接托管 WebUI 静态目录，适合裸机、NAS、小型服务器等不想额外配置 nginx 的环境。
- nginx 独立部署：nginx 服务 WebUI 静态文件，并把 `/api/*` 反向代理到 OxiDNS Next 后端，适合已有域名、HTTPS、网关或多服务统一入口的环境。

无论哪种方式，WebUI 默认使用相对后端地址 `/api`。只要 WebUI 页面和 `/api/*` 位于同一个站点 origin 下，浏览器就不需要跨域配置。

## 控制台信息架构

- 仪表盘同时展示系统概览和全部插件，并提供插件搜索、分类、表格、拓扑及创建/删除入口。插件默认按接入、编排、数据源、匹配、解析、响应处理、可观测性、系统联动和维护任务排序；手动拖拽只保存为当前浏览器的显示偏好，不会改写 YAML 执行顺序。
- 查询日志位于 `/query-log`，由 `query_recorder` 从配置的 SQLite、PostgreSQL 或 MySQL 数据库提供历史和实时流，可按域名或客户端地址关键词以及日期范围筛选。列表直接显示实际解析结果，详情展示完整响应和可自然滚动的静态执行流程。默认 `config.yaml` 使用 SQLite 并设置 7 天保留期；也可配置 Redis 短缓存加速记录列表、详情与统计 API，Redis 故障时自动回退 SQL。查询库包含客户端地址、域名与响应内容，应按敏感数据保护；Unix 上本地 SQLite 文件创建或打开时会将权限收紧为 `0600`，外部数据库则需要自行配置 TLS、账号权限、网络访问控制和备份。
- 系统日志位于 `/logs`，只展示运行和系统事件，不再混入逐查询诊断事件。
- 旧的 `/plugins` 地址保留为兼容入口，并会跳转到仪表盘的插件区域。

## 使用 Release 包内置 WebUI

官方 release 压缩包会包含已经构建好的 `webui/` 目录：

```text
oxidns-next
config.yaml
LICENSE
webui/
```

如果直接在解压目录运行 OxiDNS Next，默认配置中的 `webui.root: "./webui"` 就可以直接使用。Docker 镜像也会把同一份 WebUI 静态文件放到 `/etc/oxidns-next/webui`。

Debian 包默认服务使用 `-c /etc/oxidns-next/config.yaml -d /var/lib/oxidns-next`。因此默认配置里的 `webui.root: "./webui"` 表示 `/var/lib/oxidns-next/webui`，安装脚本会把它软链接到 `/usr/share/oxidns-next/webui`。

只有从源码构建、二次开发 WebUI，或需要自行发布静态文件到 nginx/caddy 时，才需要手动构建 WebUI。

## 手动构建 WebUI

WebUI 位于仓库的 `webui/` 目录。生产构建输出为静态目录 `webui/out`：

```bash
cd webui
pnpm install --frozen-lockfile
pnpm build
```

构建完成后，将 `out/` 目录发布到服务器上的某个目录，例如：

```bash
sudo mkdir -p /etc/oxidns-next/webui
sudo rsync -a --delete out/ /etc/oxidns-next/webui/
```

后续文档都以 `/etc/oxidns-next/webui` 作为示例静态目录。

## 方式一：后端同端口托管

这种方式只需要 OxiDNS Next 自己启动一个管理 HTTP 端口。WebUI 挂载在 `/`，管理 API 统一挂载在 `/api/*`。

```yaml
api:
  http:
    listen: "0.0.0.0:9199"
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      session_ttl_seconds: 43200
    webui:
      root: "/etc/oxidns-next/webui"
      index: "index.html"
```

启用后访问：

```text
http://服务器IP:9199/
```

WebUI 会请求同源的 `/api/health`、`/api/config`、`/api/plugins/...` 等接口。静态文件本身可以公开读取，但受保护的 `/api/*` 需要通过登录获得的 HttpOnly 会话 Cookie；修改请求还会校验 CSRF token。

默认发行配置采用 `type: accounts`。首次打开时，如果账户数据库为空，可以从监听服务所在机器创建第一个本地管理员。远程初始化应使用 `bootstrap_token_env` 注入一次性 token；初始化完成后移除该环境变量并重载。OIDC client secret 也应使用 `client_secret_env`，不要把管理员密码、引导 token、OIDC client secret 或会话 token 写入 YAML 或浏览器存储。

### 账户安全选项

- **本地登录**：密码以 Argon2 哈希保存在独立 SQLite 账户数据库中，不写入浏览器存储。
- **TOTP**：登录后可在设置页绑定验证器并保存一次性恢复码；启用后密码登录会进入第二步验证。
- **通行密钥**：需要 HTTPS 或浏览器认可的本地安全上下文。公网部署应配置 `public_url`，也可显式设置 `passkey.rp_id` 和 `passkey.origins`。
- **OIDC**：使用发现文档、state、nonce 与 PKCE。`allowed_users` 必须把身份提供方 claim 显式映射到已存在的本地账户，OIDC 不会隐式创建管理员。

字段说明：

- `api.http.webui.root`
  - WebUI 静态文件目录，必须使用 `api.http` 详写形式配置。
  - 相对路径以 OxiDNS Next 的 `-d/--working-dir` 为基准，不以配置文件所在目录为基准。
  - `api.http: "ip:port"` 简写只表示监听地址，不能挂载 WebUI。
- `api.http.webui.index`
  - 首页文件名，默认 `index.html`。
  - `/`、目录路径、以及前端路由深链未命中时都会回退到这个文件。

静态服务行为：

- `/api` 和 `/api/*` 永远进入管理 API，不会回退到 WebUI。
- 非 `/api` 的 `GET`/`HEAD` 请求会查找静态文件。
- 未命中的非 `/api` 路径会返回 `index.html`，因此刷新 `/settings` 这类前端路由可以正常工作。
- 后端会拒绝 `..`、绝对路径、非法 percent decode 等路径穿越请求。

## 方式二：nginx 独立部署

这种方式中，OxiDNS Next 只在本机监听管理 API，nginx 对外提供 WebUI 和 `/api` 反代。

OxiDNS Next 配置可以保持简单：

```yaml
api:
  http:
    listen: "127.0.0.1:9199"
    auth:
      type: accounts
      database: "./data/oxidns-next-auth.db"
      public_url: "https://oxidns-next.example.com"
```

nginx 示例：

```nginx
server {
    listen 80;
    server_name oxidns-next.example.com;

    root /etc/oxidns-next/webui;
    index index.html;

    location = /api {
        proxy_pass http://127.0.0.1:9199;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:9199;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

这里 `proxy_pass http://127.0.0.1:9199;` 会把浏览器请求的 `/api/health` 原样转发给后端，因此不要把它写成会剥离 `/api` 前缀的形式。OxiDNS Next 后端只接受 `/api/*` API 路由。

如果 nginx 负责 HTTPS，只需要把 `listen 443 ssl`、证书和 80 到 443 跳转加在 nginx 上即可；OxiDNS Next 后端仍可只监听 `127.0.0.1:9199` 明文端口，因为它不直接暴露到公网。

## WebUI 后端地址

WebUI 设置页里的后端地址推荐保持默认：

```text
/api
```

适用场景：

- 后端同端口托管 WebUI。
- nginx/caddy/网关把 `/api/*` 反代到 OxiDNS Next。
- Docker Compose 中通过统一入口容器暴露 WebUI 和 API。

只有在开发环境或临时调试时，才需要填写绝对地址，例如：

```text
http://192.168.1.10:9199/api
```

使用绝对地址时浏览器会进入跨域访问，需要后端 CORS 允许该 WebUI origin。下面的默认 CORS 推导规则只在认证关闭时使用：

- `listen: "0.0.0.0:9199"` 或 `listen: "[::]:9199"` 会允许任意 origin。
- 监听具体 IP 时，会允许同一 host 的任意 WebUI 端口。
- 显式配置 `api.http.cors.allowed_origins` 后，按浏览器 `Origin` 精确匹配。

启用账户认证后，跨源 WebUI 必须显式列出精确的 `allowed_origins`；同源部署无需额外 CORS 配置。

## 反向代理与认证注意事项

账户会话依赖 HttpOnly Cookie，推荐让 WebUI 与 `/api/*` 始终同源。OxiDNS Next 不提供公共在线 Console；不要把管理 API 暴露给任意第三方网页直连。

OIDC 和通行密钥部署必须使用最终浏览器可见的 HTTPS 地址配置 `public_url`。反向代理应保留 `Host` 并只允许可信代理访问后端监听端口；OxiDNS Next 不信任 `X-Forwarded-*` 来推导认证 origin 或协议，`public_url` 才是权威配置。

## 常见检查

- 打开 `http://服务器:9199/` 或 nginx 域名能看到 WebUI。
- 浏览器 Network 中 `/api/health` 返回 `200`，而不是请求到静态文件。
- 首次使用时 `/api/auth/session` 会报告需要初始化；完成本地管理员创建后，刷新页面应进入登录界面。
- 登录后浏览器应收到 HttpOnly 会话 Cookie；修改操作还应携带 `X-CSRF-Token`。不要把密码或会话 token 保存到浏览器 localStorage。
- 通行密钥失败时先确认页面运行在 HTTPS（或 localhost）且 `public_url`、RP ID 与 origin 匹配。
- 如果刷新 `/settings`、`/query-log` 或兼容地址 `/plugins` 后出现 404，说明静态服务缺少 `index.html` fallback；nginx 部署时确认 `try_files $uri $uri/ /index.html;` 已配置。
- 如果 nginx 反代后 `/api/health` 返回 404，检查 `proxy_pass` 是否错误剥离了 `/api` 前缀。
