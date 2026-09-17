# OxiDNS Next v0.2.0

## 🚀 发布概览

- v0.2.0 是功能版本：同步上游至 v1.5.2，并保留 OxiDNS Next 的账户、远程存储、缓存、品牌与发布链路。
- 查询日志和延时趋势不再局限于最近 1 小时；WebUI 默认展示 24 小时，并可切换 1 小时、7 天、30 天和 1 年。

## ✨ 主要亮点

- 查询趋势 API 新增日桶与真实 UTC 日历月桶，SQLite、PostgreSQL、MySQL 都会聚合所选保留范围内的全部记录、补齐空桶，并返回精确的加权平均延时和最近秩 P95。
- 查询统计及系统指标统一使用现代化 ECharts Canvas 渲染，支持响应式尺寸、深浅色主题、本地化 Tooltip、底部缩放、Ctrl + 滚轮缩放、拖动平移和移动端手势，修复旧图表的割裂、抖动和鼠标命中问题。
- 同步可信 ECS 客户端 IP、双栈专用探针、sequence 多值 mark / `set_mark`、matcher/provider 运行时控制、`response` 执行器、时区感知时间匹配和更安全的大规则集流式加载。
- RouterOS address-list 与 route 同步获得 TLS、队列合并、所有权校验、恢复与清理加固；升级下载、Windows ZIP、自升级状态和运行时生命周期也得到修复。
- WebUI 配置编辑器迁移到 CodeMirror，补齐中英文界面、日志显示、可见性轮询、更新偏好和后端账户隔离。

## ⚠️ 升级说明

- v0.1.1 的常规 YAML 配置通常可以直接升级；替换二进制前建议运行 `oxidns-next check -c <配置文件>`。
- 用户定义的插件 tag 现在必须是安全的 ASCII 路径段，且 `qs.exec.*`、`qs.match.*`、`qs.cron.*` 为 quick-setup 保留前缀；不合规的旧 tag 需要先改名。
- matcher 运行时管理 API 已由 `/enable`、`/disable` 改为 `/mode`，模式为 `normal`、`always_false` 或 `always_true`；使用旧接口的自动化需要同步调整。
- 年度图表只能展示数据库仍保留的数据。若需要完整一年历史，请提前把 query recorder 的 `retention_days` 设置为至少 `366`；已经清理的数据无法恢复。
- 趋势 API 的 `day` / `month` bucket 及 `since_ms` / `until_ms` 响应字段为向后兼容扩展，原有 `minute` / `hour` 客户端可以继续使用。

## 📦 下载与校验

- 请根据平台选择 `oxidns-next-<target>` 完整包，或 Linux musl 的 `standard` / `minimal` bundle；同时提供 x86_64 与 aarch64 Debian 软件包。
- 容器镜像发布到 `ghcr.io/ciallothu/oxidns-next:v0.2.0`；GitHub Release 资产页提供每个文件的 digest，可用于替换生产二进制前的完整性核对。
