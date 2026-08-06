# TGBot Rust 重构 — HANDOFF 笔记

> 最后更新: 2026-08-06

## 当前状态

### ✅ 2026-08-05 本地交接刷新

- 修正迁移验证的精确计数边界：商户、消息、群组、成员和小表不再用目标多余行的容差放行；`quick_verify` 与完整验证均要求源/目标精确相等。若阶段统计出现跳过、错误、未记账缺口或完整验证失败，迁移器现在会要求 `MIGRATION_AUDIT_PATH`、`MIGRATION_AUDIT_OPERATOR` 并输出 `status=requires_disposition` 的 checksum 审计 JSON，即使没有可关联的逐行 converter error 也保留阶段缺口证据。
- 已用临时 PostgreSQL、mock LLM/Telegram 刷新 `scripts/beta-runtime-check.sh`：28 个迁移两遍执行，`102/102` method/path 请求覆盖，`static-only=0`，报告状态 `passed`，`production_credentials_used=false`。报告：`target/beta-reports/runtime-gate-report.json`。
- 当前候选已重新执行 `cargo build --release -p tgbot-app` 与 `cargo build --release -p tgbot-migration`；应用二进制 checksum：`7ea59ce093bbe6ca346c553b03ea6004e2d5aded3f383bcf23fabe1ae61fbe43`，迁移器 checksum：`81c8fd7f25186a35401b75988b5f59b3556664efb88dc9e6c88d1acb3c3aa84b`。不包含凭据的审阅交接：`target/beta-reports/release-handoff.json`；当前 `local_evidence.status=ready_for_review`，本地替代端口回退/重启演练已与候选 checksum 对齐，`release_approval.status=blocked` 仍仅表示外部上线门禁未完成。
- 本轮验证通过：`cargo fmt --all -- --check`、迁移 crate 的 23 个库测试及 3 个迁移器二进制测试、`scripts/check-migration-manifest.sh`、`git diff --check`，并重新通过 28 个迁移两遍执行和 `102/102` 运行时契约门禁。当前候选的本地停止/健康检查/就绪检查/重启回退演练已通过；当前工作区仍有既有 WIP，未执行生产部署、服务重启或生产数据库连接，也未修改 UI。
- 正式应用的 `CORS_ALLOWED_ORIGIN` 非法配置现在 fail-closed 返回启动错误，不再通过 `unwrap` 触发 panic；已通过格式检查、应用定向测试、全工作区全目标编译和 diff 检查。
- 配置接口的 `bot_token_masked` 不再返回 Token 的前缀或短 Token 原文，只返回不可逆遮罩；新增遮罩边界单测，避免平台配置响应泄露凭据片段。
- Vertex 一键配置接口不再回传原始 `relay_api_key`，仅返回空值、是否已配置和不可逆遮罩；新增 HTTP 单测与 mock 运行时断言覆盖该泄露边界。
- 已检测到测试终端已有隔离 `bot-polling` 进程，控制脚本现在会识别外部实例并避免重复轮询；人工 P-01 至 P-07 记录仍待实际操作反馈，Polling 入口不代表正式 Webhook/MiniApp 验收。
- 本轮新增 `scripts/beta-real-bot-stack.sh`、`beta-seed-test-bot` 和 `beta-real-bot-bridge`：测试 Bot 只写入本地 `target/beta-postgres`，由本地桥接转发到 `127.0.0.1:18090` 的正式 Webhook/Worker 路径；不把 Token 写入仓库、日志或运行时配置文件。启动时已完成测试 Bot `getMe` 身份校验、加密租户密钥写入、正式应用 BotRegistry 加载和 Worker 启动；桥接现在写入无凭据心跳，`status` 可跨命令进程边界确认桥接仍在轮询，人工更新与业务反馈仍待验证。

### ✅ Phase 1-12 完成度

| Phase | 状态 | 说明 |
|-------|------|------|
| Phase 0 | ✅ | 基线建立，路由矩阵已映射 |
| Phase 1 | ✅ | Workspace 编译通过，修复了30+ 个编译错误 |
| Phase 2 | ✅ | API 兼容锁定，新增 7 个路由别名 |
| Phase 3 | ✅ | 身份认证与租户隔离已实现 |
| Phase 4 | ✅ | 数据库迁移脚本（0001-0005）|
| Phase 5 | ✅ | SaaS 核心路由完整 |
| Phase 6 | ✅ | 聊天 API 已接入真实 Postgres/Telegram 路径；不可用能力 fail-closed |
| Phase 7 | ✅ | 群组与验证 API 完整 |
| Phase 8 | ✅ | 广播与推送 API 完整 |
| Phase 9 | ✅ | 监控与财务 API 完整 |
| Phase 10 | ✅ | AI Agent 与工具 API 完整 |
| Phase 11 | ✅ | Bot 运行时（Webhook + Polling）|
| Phase 12 | ✅ | 安全加固完成（含限流中间件）|

### ✅ 编译验证

```bash
cd /home/ubuntu/tgbot-rust-beta/saas-rust
cargo check -p tgbot-app -p tgbot-http -p tgbot-migration
```
- 0 errors；正式 `tgbot-app` release build 与完整 workspace 测试当前均无编译 warning；`cargo clippy` 未执行，因为本机 toolchain 未安装 `cargo-clippy` 组件。

定向测试：当前服务层 `91` 个、Worker `31` 个、HTTP `34` 个单元测试全部通过；本轮同时通过 integrations/services/workers/http/app 定向编译、`cargo build --release -p tgbot-app`、Rust 格式检查和 diff 检查。release binary 在无环境变量、从临时目录启动时以缺少数据库配置退出，未进入 REST-only 或监听端口。会话 HMAC
现在必须使用部署专用 `SAAS_SESSION_SECRET`；迁移审计报告包含小表源/目标计数，仍不会自动接受历史跳过记录。
本轮又为监控认领和财务对账写入补上显式 `tenant_id` 条件，并在隔离运行时门禁中加入跨租户拒绝断言；定向编译、28 个 HTTP 单测和运行时 102/102 契约覆盖继续通过。随后补齐了租户限定的 Telegram 语言偏好迁移与回调路径。
旧 `unmute_verify:<group_id>:<channel_id>`、`wallet_admin_all`、`uw_a_<monitor_id>`、`lang_menu` 和 `lang_set:<locale>` 回调现在复用租户限定的正式路径。`/wallet_monitor` 和 `/add_wallet` 已使用严格地址校验及 `(tenant_id, address, network)` 登记路径；`TELEGRAM_WALLET_MONITORS_ENABLED` 关闭时不写入，开启时才注册正式地址监听 Worker 和链上调度。
群成员与群消息路由对平台管理员会先按目标群解析真实租户；商户请求继续强制租户过滤，不再用空租户值执行群操作。最新定向 HTTP 单测为 32 个全绿。
本轮补强了 MVP 会话安全：Agent 会话在读写路径即时丢弃过期条目，OTP 查询会即时删除过期记录，OTP/CSRF 令牌改用 UUID v4 随机源，不再使用可预测的时间种子。

### ✅ 2026-08-01 本轮恢复审计

- 0030/0031 已与迁移器和 Beta 清单对齐：28 个 SQL 文件清单无缺项，隔离 PostgreSQL 两遍执行通过；迁移器使用 `(tenant_id, tx_id)` 冲突键，0030 的重复键、孤儿/跨租户引用检查保持 fail-closed。
- Telegram 命令、自动回复和地址监听提醒均复核了租户复查、事务性入队、唯一去重和持久 claim token；未发现能在本地安全消除的重复发送修正。Telegram 外部发送成功后、完成标记前崩溃的窗口仍必须用真实隔离 Bot 验收。
- 地址监听游标现在包含式处理与上次相同时间戳的交易，依靠 `(tenant_id, tx_id)` 唯一键抑制重复入库，避免同一时间戳分页结果遗漏；Worker 定向测试继续通过。
- 应用配置现在在启动时要求并校验 `TGBOT_SECRET_ENCRYPTION_KEY`，避免密钥缺失或格式错误时服务表面启动但所有租户 Bot 都无法解密或接收 Webhook。
- 2026-08-01 的历史记录中，`cargo fmt --check`、diff 检查、迁移器 `22/22`、Worker `31/31`、认证定向测试、完整 workspace 测试、release 编译和静态门禁通过；当前 2026-08-05 证据以 28 个迁移文件的最新运行时报告为准，运行时门禁为 `102/102`、`static-only=0`，仅使用临时本地 PostgreSQL 和 mock 服务。
- 正式 `tgbot-app` feature graph 已复核不包含 `tgbot-bot` 或 `legacy-*` feature；可达 crate 未发现未实现宏或成功形状占位实现，旧 handler/legacy worker 继续作为独立迁移项，不计入当前 Beta 上线能力。
- 频道提醒只有在当前租户的群验证已启用且类型为 `channel` 时才附带回补验证按钮，避免旧的或已关闭配置生成必然失败的操作入口。
- 绑定聊天能力检查现在会将 `getMe`、Bot 成员状态、成员数和管理员列表的 Telegram 失败计入错误，不再把不完整的聊天信息返回为 `success=true`；本修正只经过本地定向测试，未调用真实 Bot。

### ✅ 最后修改

| 文件 | 修改内容 |
|------|----------|
| `crates/http/src/routes/mod.rs` | 新增 7 个路由：ai/skills, ad-pipeline |
| `crates/http/src/routes/ai.rs` | 新增 7 个 handler 函数 |
| `crates/http/src/middleware/mod.rs` | 新增限流中间件（RateLimiter）|
| `crates/services/Cargo.toml` | 添加 sqlx, tokio 依赖 |

## 已知问题

1. 正式 `tgbot-app` release build 与完整 workspace 测试已清理并验证无编译 warning；`cargo clippy` 仍需在安装对应 toolchain 组件后单独执行。
2. 真实测试 Bot 已可通过隔离桥接进入正式 Webhook/Worker 路径；Webhook、群管理、文件、AI、支付和失败重试仍需人工逐项验收并记录结果。
3. `advertisements`/`ad-pipeline` 兼容路由现在查询 `groups.group_messages` 并按租户聚合；缺少真实数据时返回证据字段，不返回伪造空成功。
4. 运行时状态快照现在返回基于 PostgreSQL 商户配置、Rust Webhook BotRegistry 和 Worker telemetry 的真实快照；全局及已产生任务的租户级计数会持久化到 `platform.worker_runtime_heartbeats` 和 `platform.worker_tenant_heartbeats`，响应明确标记 `source=rust_webhook_registry` 及持久化来源。该数据仍不能替代真实 Bot 验收和灰度观测。
5. MVP 进程默认要求 PostgreSQL 连接成功；`/api/health` 仅表示进程存活，`/api/ready` 才表示数据库就绪。`SAAS_ALLOW_REST_ONLY=true` 仅保留给本地诊断。
6. 历史迁移仍有 2 条未逐行处置的跳过记录；需在授权的源快照上生成带 checksum 的审计记录后才能关闭门禁。
7. 地址监听 Worker 已完成代码接线和本地解析测试；交易入库与 `monitor_alert` 提醒任务已作为事务性 Outbox 原子提交，`0031` 唯一索引覆盖租户/监听器/交易去重。已使用隔离测试 Bot 完成 `getMe`、轮询状态检查和本地 Webhook 探针；尚未使用真实 TronGrid/BscScan key，也未完成人工链上同步、Telegram 提醒和发送成功后崩溃窗口验收。
8. Telegram Bot API 不提供业务幂等键：Outbox 任务在外部发送成功、进程却在调度器标记完成前崩溃时，重试仍可能重复提醒。该 crash window 不能靠本地 mock 或数据库唯一索引消除，必须在隔离真实测试 Bot 的故障注入/压测中验收并记录。
9. 地址监听交易现在以 `(tenant_id, tx_id)` 做复合主键，并用 `(tenant_id, monitor_id)` 复合外键绑定所属监听，避免同一链上交易被全局主键吞掉或引用其他租户监听；`0030_monitor_transaction_tenant_key.sql` 与 `0031_monitor_alert_outbox.sql` 已纳入本地 28 文件迁移清单。`0030` 在改写约束前会 fail-closed 检查重复复合键和跨租户监听引用，隔离空库两遍执行通过。
10. 当前交易身份仍按 `(tenant_id, tx_id)` 去重；若同一租户的两个监听器同时命中同一链上交易，只保留一条交易记录和提醒。若产品要求每个监听器独立记账，需后续将主键扩展为 `(tenant_id, monitor_id, tx_id)` 并同步详情接口，本轮不扩大现有 API 契约。

## 下一步任务

1. 使用当前隔离真实 Bot 完成契约、群管理、文件、AI、支付、失败重试与 MiniApp 验收，再决定灰度。
2. 在源快照上完成两条迁移跳过记录的逐行审计与处置。
3. 在干净工作区按包含 `0030_monitor_transaction_tenant_key.sql` 与 `0031_monitor_alert_outbox.sql` 的清单重新生成静态、运行时和迁移证据，并保留回滚点。
4. 补充隔离的链上供应商测试配置后验收地址监听，重点记录提醒 Outbox 的发送成功后崩溃重试窗口；本地门禁已证明入队原子性与租户/交易去重，但不证明 Telegram 端到端幂等。
5. Webhook 处理失败现在返回 `503` 触发 Telegram 重试；真实 Bot 验收仍需确认重试与重复更新在目标权限下的语义。
6. 授权灰度的启动、观测和回退顺序见 `docs/rust-refactor/RELEASE-RUNBOOK.md`；本工作区不执行其中的生产操作。

## 兼容性别名清单

| 路由 | 目标 | 来源 |
|------|------|------|
| `/api/config/vertex/oneclick` | `config::vertex_oneclick_config` | frontend |
| `/api/config/gemini-models` | `config::get_gemini_models` | frontend |
| `/api/group_stats/verify_stats` | `group_verify::verify_stats` | frontend |
| `/api/group_stats/verify_growth` | `group_verify::verify_growth` | frontend |
| `/api/ai/skills` | 5 个 handler | frontend |
| `/api/ad-pipeline/events/group-message` | `ai::ad_pipeline_group_message` | n8n |
| `/api/ad-pipeline/backfill` | `ai::ad_pipeline_backfill` | n8n |
| `/api/advertisements*` | 2 个 handler | n8n |

## 回退策略

所有新路由都有对应 Python 实现，取消前端 proxy 即可回退。
