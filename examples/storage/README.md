# Query recorder storage examples / 查询日志存储示例

These examples run the published OxiDNS Next image with one storage choice at a time. They are intended as starting points: review the ports, credentials, image tag, retention policy, and volume layout before using them in production.

这些示例分别使用一种存储方案启动已发布的 OxiDNS Next 镜像。它们只作为部署起点；用于生产环境前，请检查端口、凭据、镜像版本、保留策略和卷布局。

## PostgreSQL

```bash
cd examples/storage
cp postgres.env.example .env
# Replace every placeholder in .env first.
docker compose --env-file .env -f postgres.compose.yaml up -d
```

The recorder URL is supplied through `OXIDNS_NEXT_QUERY_DATABASE_URL`; the example Compose file assembles it from `OXIDNS_NEXT_POSTGRES_PASSWORD`. Because the same value initializes PostgreSQL and is inserted into the URL, use a high-entropy password made from URL-safe characters.

查询记录连接 URL 通过 `OXIDNS_NEXT_QUERY_DATABASE_URL` 传入；示例 Compose 使用 `OXIDNS_NEXT_POSTGRES_PASSWORD` 组成该 URL。由于同一个值既用于初始化 PostgreSQL 又会插入 URL，请使用只包含 URL 安全字符的高强度随机密码。

## MySQL

```bash
cd examples/storage
cp mysql.env.example .env
# Replace every placeholder in .env first.
docker compose --env-file .env -f mysql.compose.yaml up -d
```

The application uses the non-root `oxidns` account. `OXIDNS_NEXT_MYSQL_ROOT_PASSWORD` is only used by the MySQL container during initialization and health checks. Use URL-safe characters for `OXIDNS_NEXT_MYSQL_PASSWORD` because Compose inserts it into the connection URL.

应用只使用非 root 的 `oxidns` 账户。`OXIDNS_NEXT_MYSQL_ROOT_PASSWORD` 仅供 MySQL 容器初始化和健康检查使用。Compose 会把 `OXIDNS_NEXT_MYSQL_PASSWORD` 插入连接 URL，因此该密码应只使用 URL 安全字符。

## Redis cache with SQLite history

```bash
cd examples/storage
cp redis.env.example .env
# Replace the bootstrap-token placeholder in .env first.
docker compose --env-file .env -f redis.compose.yaml up -d
```

This example keeps durable query history in SQLite and uses Redis for the DNS cache L2 and short-lived query-history API responses. Redis is not exposed on a host port and is not a durable source of query records. If Redis is unavailable, DNS falls back to the in-process cache/upstream and query APIs fall back to SQLite.

此示例仍将查询历史持久化到 SQLite，只让 Redis 承担 DNS 二级缓存和查询日志 API 的短缓存。Redis 不暴露宿主机端口，也不是查询记录的持久化数据源。Redis 不可用时，DNS 会回退到进程内缓存和上游，查询 API 会回退到 SQLite。

## First administrator

Each example reads `OXIDNS_NEXT_BOOTSTRAP_TOKEN` only for the initial remote administrator bootstrap. Generate a strong random value, create the administrator, then remove `bootstrap_token_env` from the selected config and remove the variable from Compose before recreating the application container.

每个示例只在远程首次创建管理员时使用 `OXIDNS_NEXT_BOOTSTRAP_TOKEN`。请生成高强度随机值；管理员创建完成后，从所选配置中删除 `bootstrap_token_env`，同时从 Compose 中删除该环境变量，再重新创建应用容器。

The WebUI is available at <http://127.0.0.1:9199>. DNS listens on host port `53` over UDP and TCP.

WebUI 位于 <http://127.0.0.1:9199>，DNS 在宿主机 `53` 端口同时监听 UDP 与 TCP。
