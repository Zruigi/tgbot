# Beta 本地 mock retry/heartbeat 快照

`scripts/beta-mock-retry-check.sh` 是一个只读、可重复的本地验收快照。它面向已经由测试操作者启动的 disposable mock stack；脚本本身不会启动、停止或重启任何服务，也不会读取 `BOT_TOKEN`。

## 显式 gate

必须显式声明 disposable mock 环境：

```bash
BETA_DISPOSABLE_MOCK=true \\
  BETA_MOCK_EXPECTED_GET_UPDATES_FAILURES=1 \\
  bash scripts/beta-mock-retry-check.sh
```

脚本拒绝非本机 URL（只接受 `127.0.0.1` 或 `localhost`），因此不会把请求发到生产 Telegram 或公网服务。默认读取：

- mock Telegram：`http://127.0.0.1:18092`
- webhook origin 的 GET-only 探针：`http://127.0.0.1:18090/healthz`
- bridge heartbeat：`/tmp/tgbot-beta-real-bot/bridge.heartbeat`

可通过 `BETA_MOCK_TELEGRAM_URL`、`BETA_MOCK_WEBHOOK_PROBE_URL` 和 `BETA_MOCK_HEARTBEAT_FILE` 指向另一个本机测试实例。

## 检查内容

脚本只执行以下读取：

1. `GET /health`：确认 mock Telegram 可达。
2. `GET /calls`：读取 `getUpdates` 的 attempt 序列。mock 启动时应使用
   `MOCK_TELEGRAM_GET_UPDATES_FAILURES=1`；序列 `1, 2` 表示一次配置的瞬时失败后出现重试。脚本不会调用清空 calls 的 `DELETE /calls`。
3. 读取 heartbeat 文件内容和 mtime，默认要求在 90 秒内。
4. 对本地 origin 执行一次 GET-only HTTP 探针。它只证明 origin 可达，不发送 Telegram update、不推导 webhook secret，也不替代 `beta-real-bot-stack.sh probe` 的 POST webhook 语义探针。

因为 `/calls` 只记录尝试次数而不记录响应体，retry 结论依赖 mock 进程外部配置的 failure count；为得到可重复的 `1 -> 2` 快照，请使用全新的 mock 进程或确认其 calls 尚未被旧运行污染。

## 报告

成功或失败都会写入：

```text
target/beta-reports/beta-mock-retry-check.json
```

报告明确记录 `credentials=none_read_or_injected`、`mutating_requests=false` 和 `services_started=false`。该快照不能替代真实 Telegram/provider 语义、真实 Bot 人工验收、迁移 skip 审计或发布灰度。


