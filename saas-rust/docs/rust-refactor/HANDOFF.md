# TGBot Rust 重构 — HANDOFF 笔记

> 最后更新: 2026-07-27

## 当前状态

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
cd D:\tmp\saas-rust
cargo check
```
- 0 errors, ~30 warnings (unused imports, dead code 等轻微警告)

### ✅ 最后修改

| 文件 | 修改内容 |
|------|----------|
| `crates/http/src/routes/mod.rs` | 新增 7 个路由：ai/skills, ad-pipeline |
| `crates/http/src/routes/ai.rs` | 新增 7 个 handler 函数 |
| `crates/http/src/middleware/mod.rs` | 新增限流中间件（RateLimiter）|
| `crates/services/Cargo.toml` | 添加 sqlx, tokio 依赖 |

## 已知问题

1. **编译警告** ~30 个 - 除无用的 import 外无风险
2. 真实测试 Bot 尚未完成 Webhook、群管理、文件、AI、支付和失败重试验收。
3. `advertisements`/`ad-pipeline` 兼容路由现在查询 `groups.group_messages` 并按租户聚合；缺少真实数据时返回证据字段，不返回伪造空成功。
4. 运行时状态快照目前明确返回 501（使用 worker telemetry），不再伪造空列表。

## 下一步任务

1. 完成真实 Bot 契约与 MiniApp 验收，再决定灰度。
2. 更新根目录 `CLAUDE.md`（如需要）。
3. 持续补充新增路由的测试和迁移跳过审计。

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
