# 隔离测试 Bot 人工验收

本文只用于一次性测试 Bot 和 Beta 工作区，不代表生产验收。不得把 Bot token 写入命令历史、日志、截图、Issue 或仓库；反馈中只保留 Bot 用户名、时间、脱敏后的更新内容和问题编号。

## 启动

在已由受控 secret 注入 `BOT_TOKEN` 的测试终端运行：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-test-bot-polling.sh
```

该入口不连接数据库，使用 `bot-polling` 开发/测试二进制。它适合先验证基础命令，不具备租户隔离；完整 Beta 验收必须使用独立数据库和 Rust Webhook 服务。

## Polling 基础命令

| 编号 | 操作 | 预期 | 结果 |
|------|------|------|------|
| P-01 | `/start` | 返回欢迎文本和当前菜单 | ☐ |
| P-02 | `/start test_payload` | 返回明确的未执行/不支持提示，不静默伪造业务成功 | ☐ |
| P-03 | `/help` | 返回当前支持的命令说明 | ☐ |
| P-04 | `/unknown` | 返回统一的未知命令提示 | ☐ |
| P-05 | `/start@其他Bot` | 忽略，不回复 | ☐ |
| P-06 | 普通文本 | 忽略，不回复 | ☐ |
| P-07 | `/wallets` | 明确提示不支持，不访问租户业务数据 | ☐ |

## Webhook/MVP 验收

如需让测试 Bot 进入完整 Beta Webhook/Worker 路径，可在已注入 `BOT_TOKEN` 的测试终端运行：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-real-bot-stack.sh start
```

启动后可用无副作用的本地探针验证派生 Webhook 密钥和正式 Rust 路由：

```bash
BETA_TEST_BOT_ENABLED=true bash scripts/beta-real-bot-stack.sh probe
```

该探针只发送一个不含消息内容的本地 `update_id`，不会向 Telegram 发送消息；它不能替代人工 Bot 语义验收。

该入口只使用 `target/beta-postgres`、`127.0.0.1:18090` 和本地加密密钥；它会将 Telegram `getUpdates` 转发到本地 Rust Webhook，并启用命令与自动回复 Worker。停止时运行：

```bash
bash scripts/beta-real-bot-stack.sh stop
```

它不会连接生产数据库、修改生产服务或写入仓库；`BETA_CLEAR_WEBHOOK=true` 只允许在确认测试 Bot 没有其他轮询进程时使用。

以下项目不能用 polling 入口替代，需在隔离 Beta 数据库和 Rust Webhook 服务上逐项记录：

- 私聊、群消息、编辑消息、重复更新和失败后 `503` 重试。
- `/start` 深链、语言偏好、自动回复、命令 Outbox 和重复入队。
- 群头像、成员数、管理员列表、群权限、成员管理和频道提醒。
- 广播文本/图片/按钮、失败汇总、文件代理和租户隔离。
- AI Chat、工具确认、MiniApp 登录和前端 102 条契约的状态码/响应/权限。
- 地址监听、TronGrid/BscScan 测试配置、提醒 Outbox，以及发送成功后进程崩溃的重试窗口。

## 问题记录

每个问题只提交以下字段，不附 token 或完整敏感请求：

```text
编号：P-__ 或 W-__
时间（UTC）：
Bot 用户名：
测试操作：
预期结果：
实际结果：
是否可稳定复现：
脱敏日志/截图路径：
```

真实 Bot、供应商、迁移跳过审计和灰度结果必须单独进入发布审批记录，不能把本清单的勾选当作上线批准。
