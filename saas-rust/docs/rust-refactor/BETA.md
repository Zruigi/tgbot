# Rust 迁移 Beta

## 隔离方式

- 生产工作区：`/home/ubuntu/tgbot`
- Beta 工作区：`/home/ubuntu/tgbot-rust-beta`
- Beta 分支：`beta/rust-migration`，基线为 `origin/rust-refactor-v2`
- Beta 服务必须使用独立端口，例如 `SAAS_BIND_ADDR=127.0.0.1:18090`
- Beta 必须连接独立测试数据库，不得复用生产 `SUPABASE_DATABASE_URL`

## 已建立的门禁

- `frontend` 与 `static` 必须和 `origin/master` 保持一致，禁止通过修改 UI 适配后端。
- 前端使用的 API 路径必须全部注册到 Rust Router。
- Rust workspace 的全部测试和全部 target 编译必须通过。
- Supabase REST 单元测试使用本地模拟服务，不访问真实项目。

运行静态门禁：

```bash
bash scripts/beta-check.sh
```

运行本机运行时门禁（会且只会重建 `tgbot_rust_beta` 数据库，并占用 `127.0.0.1:18090`）：

```bash
bash scripts/beta-runtime-check.sh
```

运行时门禁会从空库按清单顺序执行两遍全部 22 个迁移文件（最高编号 `0026`），启动独立 Rust 服务、本机模拟 LLM 与模拟 Telegram API，并验证租户隔离、Webhook 私聊/群消息/编辑消息和成员状态入库、实时广告扫描 outbox、私聊命令与租户自动回复、聊天与广播文本/图片投递、广播按钮和失败汇总、频道提醒到期删除、定时推送调度、文件代理、Webhook 密钥、动态路由、金额精度、核销审计、PDF/XLSX 下载、AI Chat/OpenAI 兼容接口、AI Skills、广告识别、潜客捕获与筛选、群组同步分配和群管理 Telegram 操作。迁移后执行只读 schema 断言，验证治理核心表（`groups.ad_group_policies`、`groups.ad_scan_jobs`、`groups.ad_governance_clusters` 等）、实时 outbox trigger 和 0024 修复函数（`auth.is_platform_admin`、`auth.current_user_is_platform_admin`）存在。测试进程通过 `HTTP_RATE_LIMIT_REQUESTS_PER_MINUTE=1000` 扩大完整回归容量，生产默认值仍为每 IP 每分钟 100 次。

2026-07-27 本地证据：`scripts/beta-check.sh` 通过；运行时报告
`target/beta-reports/runtime-gate-report.json` 通过，前端 102 条 method/path
契约均有本地请求 trace（`runtime_request_seen=102`、`static_only=0`），并由
命名矩阵断言状态码、响应形状、角色和租户隔离。请求 trace 只证明请求被发出，
不能替代真实 Bot 的语义验收。运行时仅使用临时 PostgreSQL、模拟 LLM/Telegram
和本地测试凭据，不代表真实 Bot 或生产数据库验收；编译仍有既有 warning。

占位与死代码的正式发布可达性记录见
`docs/rust-refactor/STUB-REACHABILITY-AUDIT.md`；默认 `tgbot-app` 不编译历史
standalone Bot、空内容生成/BscScan 模块或未注册的旧 Worker。启用相应
`legacy-*` feature 只用于单独审计，不能作为上线能力证据。

## 首批迁移范围

已补齐前端正在使用的 13 个 `/api/v1/monitors/finance/*` 路径，包括监听地址、交易查询、交易标注、批量标注、交易对手标签、汇总、未核销、核销、币种、会计科目、审计日志，以及 PDF/XLSX 报表下载。

交易查询使用租户条件和参数绑定；批量交易标注也增加租户隔离，避免跨租户更新。

AI Chat、OpenAI Chat Completions 和 Exec 已接入 LiteLLM 网关，不再返回伪造占位回复；广告检测已接入 Rust 本地规则引擎。AI Agent 工具调用与会话确认仍是后续迁移项。

AI Skills 已迁移到 PostgreSQL 并按租户隔离；自定义 Skill 支持保存、更新和启停。本地 Skill 只扫描 Beta 项目根目录内配置的相对子目录，拒绝绝对路径和目录穿越。

群组监听统计、同步会话创建和群组分配已迁移为租户隔离的真实数据操作，跨租户分配会被拒绝。群头像和成员数由对应租户 Bot 实时查询；频道提醒会发送真实群消息并将 12 小时有效期记录按租户落库。配置了数值频道 ID 时，回补验证按钮会通过当前租户 Webhook 检查频道成员身份，成功后解除禁言、更新验证状态并删除提醒消息；跨租户或过期配置不会触发群权限变更。

群消息发送、回复和删除已接入对应租户 Bot，并只在 Telegram 成功后更新数据库。成员禁言、解禁、封禁、踢出和管理员升降级同样采用外部成功后落库；踢出使用“封禁后解除”以允许重新加入，管理员和群主受到保护，批量操作返回逐项成功与失败结果。

Webhook 普通私聊、受监听群消息、编辑消息、入群请求和成员状态更新已按 Bot 注册租户落库；重复 Telegram 消息不会重复插入，群组归属冲突会被拒绝。到期频道提醒由后台任务按租户解析 Bot 后删除，失败采用带上限的指数退避并保留错误审计。

Webhook 私聊已接入 `/start`、`/help` 命令自动回复，支持 Bot 用户名后缀和 `/start` payload。命令运行时由默认关闭的 `TELEGRAM_COMMANDS_ENABLED` 门禁控制；只有首次成功落库的私聊消息会事务性写入 PostgreSQL Outbox，重复更新、编辑消息和群聊命令不会入队。Worker 只接收租户与入站消息标识，重新执行租户限定查询后解析命令；错误 Bot 后缀安全完成但不发送，Telegram 失败由 Job Queue 退避重试，成功回复按租户保存并关联原 Telegram 消息。

租户通用自动回复使用独立的 `/api/merchant/telegram/auto-reply` 配置接口，不修改现有 UI。商户所有者和管理员可保存默认关闭的有序规则，查看者只读；规则支持精确、包含和前缀匹配、大小写选项及长度/数量限制。默认关闭的 `TELEGRAM_AUTO_REPLIES_ENABLED` 运行时门禁控制私聊非命令文本写入独立 PostgreSQL Outbox；Worker 发送前重新按任务租户读取消息、活跃商户和当前配置，只有租户配置也开启且规则匹配时才发送。命令文本、未匹配规则、停用配置、重复消息和已回复消息均不会发送，失败保留为可退避重试任务。

受监听群消息落库后会通过 PostgreSQL Outbox 触发本地广告评分，并按默认规则和群组自定义关键词捕获潜客。策略任务只携带租户、群组和 Telegram 消息标识，Worker 会重新读取租户限定的真实消息；失败由 Job Queue 指数退避重试，未注册任务类型不会被当前 Worker 误领。潜客主键已调整为 `(tenant_id, user_id)`，同一 Telegram 用户可分别存在于多个租户；列表、标签、同步和移除操作均显式限定租户，列表支持关键词和标签筛选，写操作受角色门禁保护。

商户 Bot Token 使用 `TGBOT_SECRET_ENCRYPTION_KEY`（32 字节 hex 或 Base64）执行 AES-256-GCM 加密后落库；生产环境的 `TELEGRAM_WEBHOOK_BASE_URL` 必须为 HTTPS。聊天文本、已有 Telegram `file_id` 和 multipart 图片已接入对应租户的 Bot，文件代理只返回服务端下载后的 Data URL，不向浏览器暴露 Bot Token。

广播目标和推送任务已改为租户隔离；广播统一接口支持文本、multipart 图片、链接按钮、用户与群组混合目标、重试和真实投递结果。定时推送由应用内调度器写入 PostgreSQL Job Queue，并在执行时按租户动态解析 Bot Token、重新校验目标权限和更新运行摘要。

## 命令审计与 Polling 清理

`bot-polling.rs` 已清理为仅支持 `/start` 和 `/help`，与共享的 `tgbot_services::telegram_commands` 统一（不再使用 `teloxide::BotCommands` 派生宏）。占位 `/status` 和 `/echo` 命令已被移除，`/help` 不再声称"More coming soon…"。该二进制只有显式使用 `--features dev-polling` 才会构建，并且运行时仍要求独立的 `TELEGRAM_POLLING_ENABLED=true`；正式发布只构建 `--bin tgbot-app`。未接通业务处理的 `/start payload` 会明确返回未执行，不再静默当作普通 `/start`。

Polling 二进制使用 `TELEGRAM_POLLING_ENABLED` 门禁默认为 fail-closed：除非环境变量设置为 `true`，否则启动时立即退出。此二进制不含租户隔离，仅用于本地开发/测试。

Polling 启动后通过 Telegram `getMe` 解析当前 Bot 用户名，并直接使用共享 `TelegramCommand`，因此 `/start payload` 不会在二次转换中丢失。普通文本静默、当前 Bot 的未知命令回复安全提示、发给其他 Bot 的命令不回复。

`tgbot_services::feature_gate` 已从占位替换为基于 `current_setting()` 的真实 SQL 查询实现，并带有单元测试。

## 上线前剩余门禁

1. 将本地 102 条契约矩阵的关键响应结构、状态码、分页、权限和副作用与真实测试 Bot/前端语义回归对照；本地请求覆盖已是 102/102，但 mock 不能替代真实 Telegram、LLM 和支付供应商。
2. 用验收记录对比 Python 与 Rust 的关键响应结构、状态码、分页和权限结果。
3. 使用测试 Bot 验证 Webhook、Polling、群管理、文件、AI 和支付流程；广播已通过本机 Telegram 契约桩验证，仍需真实测试 Bot 验收。
4. 继续清除或完成可达 stub/TODO；当前旧广告管线已落库/聚合，无法在 Beta 提供真实运行时数据的接口必须返回明确错误而非空成功。
5. 先灰度代理少量测试租户，保留 Python 回退路径和数据库回滚点。
6. 对照 Python 生产语义确认其余真实命令，并接入菜单和回调。当前 `/start`、`/help` 与通用自动回复已使用耐久 Outbox，但 Telegram Bot API 不提供业务幂等键：若 Telegram 已接收发送、进程却在出站消息落库前崩溃，重试仍可能重复回复，此窗口需在 Beta 压测和运行审计中继续跟踪。
