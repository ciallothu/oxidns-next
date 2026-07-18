# OxiDNS Next v0.1.0

## 🚀 发布概览

- OxiDNS Next 首个公开版本，建立独立品牌、发布渠道与管理控制台。
- 本次重新发布的 `v0.1.0` 重点修复大型查询日志数据库下 WebUI 长时间加载或 HTTP 504 的问题，并补充 PostgreSQL、MySQL 与 Redis 支持。

## ✨ 主要亮点

- 查询日志支持 SQLite、PostgreSQL 和 MySQL；SQLite 仍是零依赖默认选项，生产环境首选 PostgreSQL，MySQL 同样受支持。
- 修复查询记录与统计请求竞争导致读取排队的问题；列表改为轻量摘要，读取具有超时与取消边界，大库下不再由已取消的查询长时间占用读取资源。
- 查询历史列表新增解析结果预览，可直接查看 IP、CNAME 等实际应答；详情页继续提供完整 Answer、Authority、Additional 与签名记录。
- 查询详情的执行路径改为紧凑的静态纵向流程，移除拖拽缩放画布与大块空白，滚轮和触摸滚动可自然到达详情底部。
- 新增可选 Redis 共享缓存：DNS `cache` 可使用 Redis 作为二级缓存，`query_recorder` 可缓存记录列表、详情与统计 API 结果。
- Redis 采用 fail-open 策略：未配置、超时或暂时不可用时，DNS 回退到进程内缓存与上游，查询日志 API 回退到 SQL 数据库；Redis 不保存唯一数据。
- 查询日志页面将记录与统计加载按顺序执行，并在切换视图时停止继续发起过期请求。
- 新增独立远程 CI，在 PostgreSQL 17、MySQL 8.4 和 Redis 7.4 服务容器中验证存储与缓存组合。

## ⚙️ 配置要点

- 在 `query_recorder.args.database.type` 中选择 `sqlite`、`postgres` 或 `mysql`；PostgreSQL / MySQL 通过 `url`、`max_connections`、`connect_timeout_ms`、`acquire_timeout_ms` 和 `query_timeout_ms` 配置。
- 先在顶层 `storage.redis` 配置 Redis URL 与键前缀，再在 DNS `cache.args.redis` 或 `query_recorder.args.api_cache` 中显式启用相应缓存。
- 数据库和 Redis URL 支持 `${VAR}` 环境变量写法；凭据应通过环境变量提供，不要提交到配置文件。

## ⚠️ 升级说明

- 根 crate 版本保持 `0.1.0`，重新发布标签为 `v0.1.0`。
- 此次是对原 `v0.1.0` 的重新发布。已部署旧版二进制或容器的用户应重新下载，或重新拉取 `v0.1.0` / `latest` 镜像。
- 旧有 SQLite 配置可以继续使用，遗留的 `query_recorder.args.path` 写法仍兼容。
- 现有 SQLite 表会自动增加轻量应答预览列，但不会批量扫描并回填旧记录；旧记录的完整响应仍可在详情中查看，新记录会直接显示解析结果。
- 不提供 SQLite 查询日志向 PostgreSQL / MySQL 的迁移工具；切换数据库时请先确认旧日志已不再需要，然后直接使用新库。
- Redis 为可选缓存层，不需要将其视为持久化数据库，也不应单独依赖 Redis 保留查询日志。

## 📦 下载与校验

- 请按平台选择 release assets 中的对应归档；容器用户可使用 `ghcr.io/ciallothu/oxidns-next:v0.1.0`。

感谢原 OxiDNS 项目与社区贡献者奠定的基础。
