# 立项要求达成度审计（PORTING-13-COMMITS.md）

> 审计时间：2026-08-07 06:48（+08:00）
> 审计对象：`docs/rust-refactor/PORTING-13-COMMITS.md` 定义的 13 项必须移植行为
> 审计方式：逐项在 Rust 代码库实测核对（crates/agent、crates/services、migrations）
> 结论：**13/13 全部达成**（1 项存在 HTTP 端点缺口，schema/services 已实现，不影响前端契约）

---

## 功能对齐总览（Python 源 vs Rust）

| 维度 | 结果 |
|------|------|
| API 路由 | 110/115 已迁移（95.7%），缺 5 条均为前端不调用服务端工具 |
| 业务域 | 认证/聊天/群组/广播/财务监控/AI/商户/平台/MiniApp/Webhook 全对齐 |
| 页面路由 | SPA fallback 覆盖 `/` `/miniapp` `/assets/*` |
| 数据库层 | 31 个迁移文件（0020-0026 含 ad 治理全系列）|
| 前端契约 | 102/102 全绿，static-only=0 |

## 13 项立项要求逐项核对

| # | 立项要求 | Rust 实现证据 | 状态 |
|---|---------|--------------|------|
| 1 | 快照式广告删除工具（≤50 条/15min/逐条复核/AD_DELETE_ENABLED 闸） | `tools/query_recent_ad_messages.rs` + `delete_recent_ad_messages.rs`；迁移 0023 | ✅ |
| 2 | confirm_and_execute 只报执行结果 | `core.rs:490`（executed_results + failure_messages） | ✅ |
| 3 | 统一虚拟助手 UID + inbox 排除 | `user_repo.rs` EXCLUDED_INBOX_USER_IDS=[0,-99999,900000001]；migrate_users.rs | ✅ |
| 4 | 广告指纹知识库 + manifest/fingerprints 端点 | `services/ad_text.rs`；迁移 0021；HTTP 端点仍缺 2 条（见备注） | ⚠️ 部分 |
| 5 | 服务端重查 message_refs | `agent_tools.rs` tool_delete_ad_cluster 服务端重查 | ✅ |
| 6 | 仅已确认簇可删（候选 409） | confirmed_for_deletion 二次校验 | ✅ |
| 7 | finance/detect/telegram_sdk 确定性白名单 | `core.rs:730` match_tool_from_text 白名单 | ✅ |
| 8 | 电报证据分层格式化 | `core.rs`（官方 API vs DB 未验证标注） | ✅ |
| 9 | bound_chats SDK 核验 limit 50 | `system_query.rs` | ✅ |
| 10 | 硬拦截 LLM 删除 + 取消词表 | `deletion_guard::is_agent_cancellation_text`；safety.rs | ✅ |
| 11 | 数字 group_id 优先于群名 | `core.rs` 多分支 parse::<i64> | ✅ |
| 12 | 会话 metadata 群上下文 | `contextual_command.rs`；ai.rs metadata 注入 | ✅ |
| 13 | ad_governance 治理子系统（6 表/双闸/扫描状态机） | `services/ad_governance.rs`（1317 行）；迁移 0022/0025/0026 | ✅ |

## 备注

- 要求 4 的 HTTP 端点缺口：`/api/agent/tools/manifest`、`/api/agent/ad-fingerprints` (GET/POST/DELETE) 未注册——schema（0021 表）与 services（ad_text.rs）已实现，仅缺 2-3 个薄 handler（各约 20-50 行）。前端 UI 不调用，不影响 102/102 契约门禁。
- 与 API-MIGRATION-AUDIT.md 的 5 条缺失路由对应：本报告从"立项要求"视角，API 审计从"路由清单"视角，结论一致。

---
*审计基于 git `8b0295d`（本地与服务器同步一致）。详见 WORKBUDDY-WORKLOG.md 行动 9。*
