# Rust 迁移 Beta

## 隔离方式

- 生产工作区：`/home/ubuntu/tgbot`
- Beta 工作区：`/home/ubuntu/tgbot-rust-beta`
- Beta 分支：`beta/rust-migration`，基线为 `origin/rust-refactor-v2`
- Beta 服务必须使用独立端口，例如 `SAAS_BIND_ADDR=127.0.0.1:18090`
- Beta 必须连接独立测试数据库，不得复用生产 `SUPABASE_DATABASE_URL`
- 数据迁移工具必须显式提供 `SQLITE_DB_PATH`，不会默认猜测或读取生产 SQLite 文件

## 已建立的门禁

- `frontend` 与 `static` 必须和 `origin/master` 保持一致，禁止通过修改 UI 适配后端。
- 前端使用的 API 路径必须全部注册到 Rust Router。
- 正式 `tgbot-app` 依赖图不得启用 `legacy-bot`、`legacy-workers` 或 `legacy-stubs`。
- Rust workspace 的全部测试和全部 target 编译必须通过。
- Supabase REST 单元测试使用本地模拟服务，不访问真实项目。
- MVP 启动默认必须建立 PostgreSQL 连接；`/api/ready` 只在数据库可用时返回成功。
- `SAAS_ALLOW_REST_ONLY=true` 仅用于本地诊断，不属于 MVP 发布配置。

运行静态门禁：

```bash
bash scripts/beta-check.sh
```

运行本机运行时门禁（会且只会重建 `tgbot_rust_beta` 数据库，并占用 `127.0.0.1:18090`）：

```bash
bash scripts/beta-runtime-check.sh
```

需要在普通本地终端保留 Beta 数据库并启动正式 Rust MVP 时，使用固定的本机隔离启动器；它只使用 `target/beta-postgres`、`127.0.0.1:15432` 和 `tgbot_rust_beta`，不读取 `.env`，不使用 `BOT_TOKEN`：

```bash
bash scripts/beta-local-app.sh migrate
bash scripts/beta-local-app.sh run
```

按 `Ctrl+C` 停止应用后，可用 `bash scripts/beta-local-app.sh stop` 停止本地 PostgreSQL；数据目录会保留，不执行删除。

验证本地候选的停止、健康检查、数据库就绪、重启和回退后不可访问状态，可运行：

```bash
bash scripts/beta-local-rollback-rehearsal.sh
bash scripts/beta-isolation-audit.sh
```

如需让隔离测试 Bot 进入人工测试轮询，必须先由测试终端注入一次性 `BOT_TOKEN`，再显式运行：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-test-bot-polling.sh
```

如果需要让 Bot 在终端退出后继续供人工测试，可使用受控后台入口；它只保存 `/tmp` 下的 PID 和日志：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-test-bot-control.sh start
bash scripts/beta-test-bot-control.sh status
bash scripts/beta-test-bot-control.sh stop
```

该入口只运行 `bot-polling` 开发/测试二进制，不连接数据库、不读取工作区 `.env`，只覆盖 `/start`、`/help`、未知命令和忽略普通文本等基础 Bot 行为；它没有租户隔离，不能代替正式 Webhook/MiniApp 验收。控制脚本会识别已从其他终端启动的同一 Beta polling 进程，避免重复轮询；外部进程必须回到原终端用 `Ctrl+C` 停止，脚本不会打印或保存 `BOT_TOKEN`。

人工验收步骤和脱敏问题记录模板见 `docs/rust-refactor/TEST-BOT-ACCEPTANCE.md`。

运行时门禁会从空库按清单顺序执行两遍全部 28 个迁移文件（最高编号 `0031`，含同编号的历史 SQL 文件），启动独立 Rust 服务、本机模拟 LLM 与模拟 Telegram API，并验证租户隔离、Webhook 私聊/群消息/编辑消息和成员状态入库、实时广告扫描 outbox、私聊命令与租户自动回复、语言偏好、聊天与广播文本/图片投递、广播按钮和失败汇总、频道提醒到期删除、定时推送调度、文件代理、Webhook 密钥、动态路由、金额精度、核销审计、PDF/XLSX 下载、AI Chat/OpenAI 兼容接口、AI 原生 function calling、AI Skills、广告识别、潜客捕获与筛选、群组同步分配和群管理 Telegram 操作。迁移后执行只读 schema 断言，验证治理核心表（`groups.ad_group_policies`、`groups.ad_scan_jobs`、`groups.ad_governance_clusters` 等）、实时 outbox trigger、job claim token、Worker 持久化遥测表（`platform.worker_runtime_heartbeats`、`platform.worker_tenant_heartbeats`）、`monitor_transactions` 的 `(tenant_id, tx_id)` 复合主键和 `(tenant_id, monitor_id)` 复合外键，并用临时数据验证同一 `tx_id` 可跨租户共存、跨租户监听器引用被拒绝、提醒 Outbox 重复入队被唯一索引去重；同时验证 0024 修复函数（`auth.is_platform_admin`、`auth.current_user_is_platform_admin`）存在。应用进程从无 `.env` 的临时目录以 `env -i` 空环境启动，只注入本地数据库和 mock 服务配置；报告同时记录 HEAD 和工作区是否干净。测试进程通过 `HTTP_RATE_LIMIT_REQUESTS_PER_MINUTE=1000` 扩大完整回归容量，生产默认值仍为每 IP 每分钟 100 次。

2026-08-01（Asia/Shanghai）本轮本地证据：`scripts/beta-runtime-check.sh`
通过；运行时报告
`target/beta-reports/runtime-gate-report.json` 通过，前端 102 条 method/path
契约均有本地请求 trace（`runtime_request_seen=102`、`static_only=0`），并由
命名矩阵断言状态码、响应形状、角色和租户隔离。请求 trace 只证明请求被发出，
不能替代真实 Bot 的语义验收。运行时仅使用临时 PostgreSQL、模拟 LLM/Telegram
和本地测试凭据，不代表真实 Bot 或生产数据库验收；正式 release build 与完整 workspace 测试本轮均无编译 warning。
schema probe 同时覆盖复合交易键、复合外键和提醒 Outbox 唯一去重；`0030` 迁移在约束改写前会拒绝重复复合键和跨租户监听引用。
`beta-handoff-check.sh` 会单独记录 UI/static 门禁；父目录 `frontend` 当前存在继承的
未提交改动，本轮未触碰该目录，因此当前报告明确标记
`ui_gate=blocked_preexisting_worktree_changes`。最终发布证据必须在干净工作区重新生成；带
`working_tree_dirty=true` 的报告只用于开发中回归。
运行时状态接口现在同时提供进程内调度器和 PostgreSQL 持久化的全局/租户级 Worker
计数、最近轮询与最近完成/失败时间；`durable_worker_telemetry_available=true`
表示最近两分钟内存在持久化心跳，租户级字段只在该租户实际出现任务后提供。它仍不
替代外部监控、真实 Bot 验收或灰度期间的持续观测。

占位与死代码的正式发布可达性记录见
`docs/rust-refactor/STUB-REACHABILITY-AUDIT.md`；默认 `tgbot-app` 不编译历史
standalone Bot、空内容生成模块或未注册的旧 Worker；`bscscan` 现在是默认关闭的
正式链上查询客户端，不属于历史 stub。启用相应
`legacy-*` feature 只用于单独审计，不能作为上线能力证据。

2026-08-05（Asia/Shanghai）本地交接刷新：迁移验证已收紧为精确源/目标计数，
并在统计缺口没有逐行错误时也输出带 checksum 的 `requires_disposition` 审计
报告。临时 PostgreSQL + mock 服务运行门禁再次通过（28 个迁移两遍、
`102/102`、`static-only=0`）；release 二进制和不含凭据的交接报告分别记录于
`target/release/tgbot-app` 与 `target/beta-reports/release-handoff.json`。
本地交接可审阅，但两条历史跳过、真实 Bot/供应商、灰度观测和回滚演练仍是
正式放行门禁；本工作区不执行这些外部操作。

## 首批迁移范围

已补齐前端正在使用的 13 个 `/api/v1/monitors/finance/*` 路径，包括监听地址、交易查询、交易标注、批量标注、交易对手标签、汇总、未核销、核销、币种、会计科目、审计日志，以及 PDF/XLSX 报表下载。

交易查询使用租户条件和参数绑定；批量交易标注也增加租户隔离，避免跨租户更新。

AI Chat、OpenAI Chat Completions 和 Exec 已接入 LiteLLM 网关，不再返回伪造占位回复；普通 AI Chat 已切换到 Rust `AIAgent`，支持原生 LiteLLM function calling、租户上下文和按可信认证主体隔离的进程内待确认会话，确认失败会保留待确认状态并 fail-closed。广告检测已接入 Rust 本地规则引擎。真实 Bot 的破坏性操作、LLM 语义和会话重启恢复仍需 Beta 验收。

AI Agent 构建时会按当前租户解析已注册 Bot，并将租户限定的 Telegram 客户端与 LiteLLM 客户端传入广播工具；Agent 广播支持 Telegram 图片 file_id/URL、HTML 正文、链接按钮和翻译，未解析到活跃 Bot 或翻译服务时保持 fail-closed，不回退到全局 Bot 或跨租户客户端。

AI Skills 已迁移到 PostgreSQL 并按租户隔离；自定义 Skill 支持保存、更新和启停。本地 Skill 只扫描 Beta 项目根目录内配置的相对子目录，拒绝绝对路径和目录穿越。Rust `AIAgent` 已注册租户限定的 `system_query` 和 `query_ad_clusters`；`skill_manage` 仅向商户所有者、管理员和平台管理员开放，并要求显式租户上下文。直接广告聚合查询同样拒绝平台侧的无租户查询。

`system_query` 的 `bound_chats` 已按租户合并监听群与广播目标，区分群组、频道和仅广播目标；`active_groups` 使用已落库的租户成员记录计算成员数，不再返回固定的伪造零值。查询结果采用确定性中文摘要，空数据和失败状态保持可审计语义。

群组监听统计、同步会话创建和群组分配已迁移为租户隔离的真实数据操作，跨租户分配会被拒绝。群头像和成员数由对应租户 Bot 实时查询；频道提醒会发送真实群消息并将 12 小时有效期记录按租户落库。配置了数值频道 ID 时，回补验证按钮会通过当前租户 Webhook 检查频道成员身份，成功后解除禁言、更新验证状态并删除提醒消息；跨租户或过期配置不会触发群权限变更。

群消息发送、回复和删除已接入对应租户 Bot，并只在 Telegram 成功后更新数据库。成员禁言、解禁、封禁、踢出和管理员升降级同样采用外部成功后落库；踢出使用“封禁后解除”以允许重新加入，管理员和群主受到保护，批量操作返回逐项成功与失败结果。

Agent 消息存在性探测会先验证目标群属于当前租户，再从 `tenant_saas.tenant_configs.revoke_sync_chat_id` 读取校对信息群；请求中的 `staging_chat_id` 只能作为一致性校验，不能指定其他租户或未配置的目标群。

Webhook 普通私聊、受监听群消息、编辑消息、入群请求和成员状态更新已按 Bot 注册租户落库；重复 Telegram 消息不会重复插入，群组归属冲突会被拒绝并安全确认，不写入当前租户，也不因确定性的跨租户更新触发 Telegram 重试。到期频道提醒由后台任务按租户解析 Bot 后删除，失败采用带上限的指数退避并保留错误审计。租户隔离拒绝路径会写入 `tenant_audit_logs`，记录角色、动作、资源和请求路径。

Webhook handler 的持久化或外部处理失败（包括 PostgreSQL 不可用）会返回 `503` 而不是确认成功，允许 Telegram 重试；入库幂等键、Outbox 唯一索引和任务 claim token 负责限制重试造成的重复副作用。错误响应只返回通用服务不可用信息，不向 Telegram 暴露内部错误或凭据。

Webhook 私聊已接入 `/start`、`/help`、`/wallets` 命令自动回复，支持 Bot 用户名后缀、已配置的 `channel_<group_id>_<channel_id>` `/start` 深链和租户限定的 `unmute_<group_id>` 解禁深链。频道深链会先按租户校验群组配置，再查询频道成员；只有 Telegram 成功确认关注后才解禁并落库，未关注时返回重新验证按钮，未知 payload 仍 fail-closed。`/start` 菜单、地址监听列表、取消监听和返回主菜单回调均按当前 Bot 租户及 Telegram 用户限定；命令运行时由默认关闭的 `TELEGRAM_COMMANDS_ENABLED` 门禁控制；只有首次成功落库的私聊消息会事务性写入 PostgreSQL Outbox，重复更新、编辑消息和群聊命令不会入队。Worker 只接收租户与入站消息标识，重新执行租户限定查询后解析命令；错误 Bot 后缀安全完成但不发送，Telegram 失败由 Job Queue 退避重试，成功回复按租户保存并关联原 Telegram 消息。

`/wallet_monitor <address>` 和 `/add_wallet <address>` 已接入同一个租户限定的 `address_monitors` 登记路径，支持格式严格校验、同租户重复登记和停用后重新启用；`wallet_monitor` 回调只提供输入提示，不绕过命令登记。`TELEGRAM_WALLET_MONITORS_ENABLED` 默认关闭，关闭时不写入地址监听；开启后应用才注册正式 `AddressMonitorWorker`、`monitor_check` 调度和 TronGrid/BscScan 客户端。链上同步从登记时间建立游标，交易入库与 `monitor_alert` 提醒 Outbox 在同一 PostgreSQL 事务中提交，并按租户解析 Bot 发送提醒；隔离测试 Bot 已完成 API 身份和本地 Webhook 探针验收，但尚未使用真实链上 API key 或完成地址监听、提醒和故障窗口验收，因此这部分仍不是上线证据。

订阅验证回调会执行租户校验、频道成员检查、解禁和提醒清理；地址监听相关的已迁移回调执行用户与租户双重限定；其他尚未迁移的旧菜单回调只返回明确的 Beta“不支持此操作”提示并记录审计日志，不伪造业务成功。旧菜单的其余语义仍需真实测试 Bot 与 Python 对照验收后再接入。

租户通用自动回复使用独立的 `/api/merchant/telegram/auto-reply` 配置接口，不修改现有 UI。商户所有者和管理员可保存默认关闭的有序规则，查看者只读；规则支持精确、包含和前缀匹配、大小写选项及长度/数量限制。默认关闭的 `TELEGRAM_AUTO_REPLIES_ENABLED` 运行时门禁控制私聊非命令文本写入独立 PostgreSQL Outbox；Worker 发送前重新按任务租户读取消息、活跃商户和当前配置，只有租户配置也开启且规则匹配时才发送。命令文本、未匹配规则、停用配置、重复消息和已回复消息均不会发送，失败保留为可退避重试任务。

受监听群消息落库后会通过 PostgreSQL Outbox 触发本地广告评分，并按默认规则和群组自定义关键词捕获潜客。策略任务只携带租户、群组和 Telegram 消息标识，Worker 会重新读取租户限定的真实消息；失败由 Job Queue 指数退避重试，未注册任务类型不会被当前 Worker 误领。潜客主键已调整为 `(tenant_id, user_id)`，同一 Telegram 用户可分别存在于多个租户；列表、标签、同步和移除操作均显式限定租户，列表支持关键词和标签筛选，写操作受角色门禁保护。

商户 Bot Token 使用 `TGBOT_SECRET_ENCRYPTION_KEY`（32 字节 hex 或 Base64）执行 AES-256-GCM 加密后落库；生产环境的 `TELEGRAM_WEBHOOK_BASE_URL` 必须为 HTTPS。会话 HMAC 使用必需的部署专用 `SAAS_SESSION_SECRET`，缺失或短于 32 字节时签发和校验均 fail-closed。聊天文本、已有 Telegram `file_id` 和 multipart 图片已接入对应租户的 Bot，文件代理只返回服务端下载后的 Data URL，不向浏览器暴露 Bot Token。

广播目标和推送任务已改为租户隔离；广播统一接口支持文本、multipart 图片、链接按钮、用户与群组混合目标、重试和真实投递结果。定时推送由应用内调度器写入 PostgreSQL Job Queue，并在执行时按租户动态解析 Bot Token、重新校验目标权限和更新运行摘要。

## 命令审计与 Polling 清理

`bot-polling.rs` 仍仅支持 `/start` 和 `/help`，与共享的 `tgbot_services::telegram_commands` 统一解析但不会启用租户限定的 `/wallets` 菜单（不再使用 `teloxide::BotCommands` 派生宏）。占位 `/status` 和 `/echo` 命令已被移除，`/help` 不再声称"More coming soon…"。该二进制只有显式使用 `--features dev-polling` 才会构建，并且运行时仍要求独立的 `TELEGRAM_POLLING_ENABLED=true`；正式发布只构建 `--bin tgbot-app`。未接通业务处理的 `/start payload` 会明确返回未执行，不再静默当作普通 `/start`。

Polling 二进制使用 `TELEGRAM_POLLING_ENABLED` 门禁默认为 fail-closed：除非环境变量设置为 `true`，否则启动时立即退出。此二进制不含租户隔离，仅用于本地开发/测试。

Polling 启动后通过 Telegram `getMe` 解析当前 Bot 用户名，并直接使用共享 `TelegramCommand`，因此 `/start payload` 不会在二次转换中丢失。普通文本静默、当前 Bot 的未知命令回复安全提示、发给其他 Bot 的命令不回复。

`tgbot_services::feature_gate` 已从占位替换为基于 `current_setting()` 的真实 SQL 查询实现，并带有单元测试。

## 上线前剩余门禁

1. 将本地 102 条契约矩阵的关键响应结构、状态码、分页、权限和副作用与真实测试 Bot/前端语义回归对照；本地请求覆盖已是 102/102，但 mock 不能替代真实 Telegram、LLM 和支付供应商。
2. 用验收记录对比 Python 与 Rust 的关键响应结构、状态码、分页和权限结果。
3. 使用测试 Bot 验证 Webhook、Polling、群管理、文件、AI 和支付流程；广播已通过本机 Telegram 契约桩验证，仍需真实测试 Bot 验收。
4. 可达 stub/TODO 的静态审计已完成：正式 `tgbot-app` graph 不包含 `tgbot-bot` 或 `legacy-*` feature，当前可达 crate 没有未实现宏或成功形状占位实现；旧广告管线已落库/聚合，无法在 Beta 提供真实运行时数据的接口返回明确错误而非空成功。剩余旧 handler 和 legacy worker 不属于正式 Beta graph，仍需独立迁移时再处理。
5. 先灰度代理少量测试租户，保留 Python 回退路径和数据库回滚点。
6. 对照 Python 生产语义确认其余真实命令，并接入菜单和回调。当前 `/start`、`/help`、`/wallets`、`/wallet_monitor`、`/add_wallet`、频道验证深链、`unmute_<group_id>` 解禁深链、`unmute_verify:<group_id>:<channel_id>` 旧验证回调、地址监听列表/取消/租户管理员列表/取消/返回回调、租户限定的 `lang_menu`/`lang_set:<locale>` 语言偏好和通用自动回复已使用耐久 Outbox 或租户限定的 Webhook 处理；地址监听命令和链上 Worker 仍受 `TELEGRAM_WALLET_MONITORS_ENABLED` 默认关闭门禁控制，真实供应商和 Bot 语义尚未验收。Outbox 认领 token 已持久化并在完成、失败和超时重取时校验，避免旧 worker 覆盖新 worker 的结果。Telegram Bot API 不提供业务幂等键：若 Telegram 已接收发送、进程却在出站消息落库前崩溃，重试仍可能重复回复，此窗口需在 Beta 压测和运行审计中继续跟踪。
7. 对历史迁移报告中的两条跳过记录生成逐行审计并作出修复或明确处置；所有迁移阶段现在会保留逐行失败并要求生成带 checksum 的审计 JSON。当前迁移程序会对任何跳过、错误或未记账源记录直接失败，不能再用总量容差放行。
