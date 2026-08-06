# Rust Beta 发布与回退 Runbook

本文只描述授权运维在隔离环境或获准灰度环境中的操作顺序。本工作区不会执行生产发布、服务重启或生产数据库连接；本地门禁只使用临时数据库和 mock 服务，不读取凭据。

## 当前候选

- 候选分支：`beta/rust-migration`
- 正式二进制：`target/release/tgbot-app`
- Beta 默认监听：`127.0.0.1:18090`
- 正式进程必须显式设置 `SAAS_BIND_ADDR`，不得依赖 Beta 默认端口
- `SAAS_ALLOW_REST_ONLY=false` 必须保持关闭
- Python 代理和现有 UI 保持不变，作为灰度期间的回退路径

## 本地交接（不发布）

以下命令只使用临时 PostgreSQL、mock LLM/Telegram 和本地构建产物，不读
`.env`、不连接生产数据库、不修改 UI，也不执行部署：

```bash
cd /home/ubuntu/tgbot-rust-beta/saas-rust
bash scripts/beta-runtime-check.sh
cargo build --release -p tgbot-app -p tgbot-migration
bash scripts/beta-handoff-check.sh
bash scripts/beta-local-rollback-rehearsal.sh
bash scripts/beta-isolation-audit.sh
```

## 隔离测试 Bot 人工验收

在测试终端通过受控 secret 注入 `BOT_TOKEN` 后运行，不要把 token 写入命令历史、仓库或日志：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-test-bot-polling.sh
```

后台人工测试可使用：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-test-bot-control.sh start
bash scripts/beta-test-bot-control.sh status
bash scripts/beta-test-bot-control.sh stop
```

该轮询入口只用于一次性测试 Bot 的基础命令人工验收；它没有租户隔离，不属于生产发布路径，也不替代正式 Rust Webhook、数据库租户路由、MiniApp、供应商和重试验收。控制脚本会识别外部终端已启动的 polling 进程并拒绝重复启动；外部进程回原终端按 `Ctrl+C` 停止。

测试用例、Webhook/MVP 验收范围和脱敏反馈格式见 `docs/rust-refactor/TEST-BOT-ACCEPTANCE.md`。

`beta-handoff-check.sh` 会复核迁移清单、前端 method/path 注册、格式、迁移
crate 测试和 diff，并记录 `target/release/tgbot-app` 的 SHA-256。它生成
`target/beta-reports/release-handoff.json`，将 `local_evidence` 与
`release_approval` 分开；`local_evidence.status=ready_for_review` 不等同于
正式放行。当前交接报告记录了 `102/102`、`static-only=0`、28 个迁移两遍
执行及 disposable local credentials only。

## 授权发布前检查

1. 在干净的候选工作区执行 `cargo build --release -p tgbot-app`，再执行 `sha256sum target/release/tgbot-app` 和 `git rev-parse HEAD`，保存二进制 checksum 和 commit ID。
2. 在独立数据库或获准目标数据库的备份副本上按顺序执行全部迁移；`0030_monitor_transaction_tenant_key.sql` 会在改写约束前 fail-closed 检查同租户重复交易键、重复监听器键和跨租户监听器引用，`0031_monitor_alert_outbox.sql` 之后检查唯一索引。
3. 以专用迁移角色运行迁移并保留两条历史跳过记录的逐行 checksum 审计；没有处置记录时不得放行。
4. 通过安全的进程管理器注入 `SUPABASE_DATABASE_URL` 或 `DATABASE_URL`、`SAAS_SESSION_SECRET`、`INTERNAL_SAAS_SECRET`、`TGBOT_SECRET_ENCRYPTION_KEY` 和供应商配置。凭据只进受控 secret store，不写入命令行、仓库、Runbook 或日志。
5. 确认正式配置未开启 `SAAS_ALLOW_REST_ONLY`、未意外开启 `TELEGRAM_COMMANDS_ENABLED`、`TELEGRAM_AUTO_REPLIES_ENABLED` 或 `TELEGRAM_WALLET_MONITORS_ENABLED`；每项能力应按验收范围逐项开启。
6. 在隔离真实测试 Bot 和链上供应商配置上完成 Webhook、命令、群管理、文件、AI、支付和地址提醒验收；本地 mock 报告不能替代此步骤。

## 灰度顺序

1. 先启动独立 Rust 进程，不占用 Python 当前监听端口。
2. 验证 `/api/health` 和 `/api/ready`；前者只证明进程存活，后者必须证明数据库连接可用。
3. 仅将获准的测试租户或测试流量代理到 Rust；生产 UI 不做改版或接口替换。
4. 观察 HTTP 5xx、租户拒绝审计、Worker heartbeat、Outbox dead job、Telegram 发送失败和数据库连接池指标。
5. 验收通过后再扩大租户范围；任何异常先停止扩大范围并保留 Python 回退。

## 回退顺序

1. 先将代理流量切回 Python，确认旧路径健康。
2. 停止 Rust 候选进程，保留日志、commit、二进制 checksum 和迁移审计。
3. 不直接反向执行已应用的破坏性迁移；按数据库备份和授权 DBA 的恢复方案处理数据。
4. 若只回退应用版本，保留兼容迁移和 Outbox 数据，避免旧进程误读新 schema。
5. 记录触发时间、租户范围、错误类型和最终数据核对结果，再决定是否重试灰度。

## 当前未放行项

- 已使用一次性隔离测试 Bot 完成 `getMe`、轮询状态检查和本地 Webhook 探针；未使用 TronGrid/BscScan 或其他生产供应商凭据，真实业务语义仍尚未验收。
- 两条历史迁移跳过记录尚无授权源快照逐行处置和 checksum。
- Telegram API 没有业务幂等键，外部发送成功后进程崩溃的重复提醒窗口仍需真实故障注入验收。
- 当前工作区包含未提交 WIP；正式发布证据必须从干净候选工作区重新生成。
