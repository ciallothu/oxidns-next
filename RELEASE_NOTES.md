# OxiDNS Next v0.1.1

## 🚀 发布概览

- v0.1.1 是兼容性补丁版本，重点修复 WebUI 深色模式下图表提示框不可读，以及切换图表和快速悬停时动画抖动、追赶的问题。
- 本版本同时同步 React Flow 画布主题，完善默认 PostgreSQL / Redis Compose 部署栈，并加入持续的 Rust 与 WebUI 依赖审计。

## ✨ 主要亮点

- 查询统计 Tooltip 显式使用当前主题的 popover 前景、背景与边框色，深色模式下文字恢复清晰可读。
- 关闭查询统计 Tooltip、Bar、Pie 与 Line 的过渡动画，消除切换标签、时间范围、刷新或快速移动鼠标时的重复、反向与闪动效果。
- 插件拓扑和 Sequence 编辑画布跟随实际深浅色主题，节点、控件与画布配色保持一致。
- 根配置默认使用 PostgreSQL 保存查询历史，并启用 Redis DNS 二级缓存和查询 API 缓存；根 Compose 提供 PostgreSQL 17、Redis 7.4、健康检查、持久卷、内部网络与 `.env.example`。
- 更新 Next.js、Docusaurus 及受安全公告影响的依赖解析，新增 Security Audit workflow，对完整 WebUI、文档与 Rust 依赖树持续审计。
- TLS PEM 加载改为直接使用 `rustls-pki-types`，移除不再维护的 `rustls-pemfile` 直接依赖，现有证书与私钥配置无需修改。

## ⚠️ 升级说明

- 根 crate 版本为 `0.1.1`，发布标签为 `v0.1.1`；现有运行配置可以直接升级，没有新的必填 schema 字段。
- 仓库根 `config.yaml` 的默认部署已改为 PostgreSQL 与 Redis。使用根 Compose 时，请先复制 `.env.example` 为 `.env`，设置两个 URL-safe 密码，再启动完整栈。
- SQLite、MySQL 和自定义单容器部署仍受支持；请保留自己的配置，不要直接用新的根配置覆盖。查询历史只持久化到 SQL 数据库，Redis 仍是可丢弃缓存。
- RustSec 对 `RUSTSEC-2023-0071` 保留一项经过评估的例外：当前 OIDC / MySQL 依赖路径只执行 RSA 公钥操作，该公告影响私钥计时，且暂无已修复版本。

## 📦 下载

- 请根据平台和 bundle 选择 GitHub Release 中对应的 archive 或 Debian 软件包。
- 容器用户可使用 `ghcr.io/ciallothu/oxidns-next:v0.1.1`。
- 本版本未随附项目生成的 checksum、SBOM 或 provenance；请仅通过项目 GitHub Release 或 GHCR 官方发布渠道获取产物。
