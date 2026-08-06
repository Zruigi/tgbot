# Rust API 迁移对照报告（Python 主控 → Rust 架构）

> 生成时间：2026-08-07 06:30（+08:00）
> 对照基准：Python 源 `/home/ubuntu/tgbot/web_admin.py`（128 条路由）vs Rust `/home/ubuntu/tgbot-rust-beta/saas-rust/crates/http/src/routes/mod.rs`（155 条 route 注册）
> 结论：**Python 主控程序的 API 已基本完整迁移到 Rust，前端契约 102/102 全绿，仅 4 个服务端内部工具接口未迁移（前端不调用，不影响上线）**

---

## 一、总量对照

| 维度 | Python（web_admin.py） | Rust（routes/mod.rs） | 说明 |
|------|----------------------|----------------------|------|
| 路由注册 | 128 | 155（含 method 展开） | Rust 数量更多，因新增了 finance/v1、openai、verify 等接口 |
| API 路由（/api/*） | 115 | 150+ | 归一化去重后对比 |
| 前端契约（method+path） | — | **102/102 全绿** | beta-runtime-check 门禁通过 |
| static-only 用例 | — | 0 | 所有前端调用都在运行时真实请求过 |

## 二、Python 有但 Rust 缺失的路由（共 11 条，其中 6 条为页面路由）

### 已覆盖的页面路由（6 条，无需迁移）
| Python 路由 | 状态 |
|------------|------|
| `/`、`/miniapp`、`/favicon.ico`、`/{path_info}` | ✅ Rust 已实现 SPA fallback（FRONTEND_DIST_DIR 静态服务，2026-08-07 提交） |
| `/admin`、`/admin/{path_info}` | ✅ 生产 Python 管理页，不在 Rust 重构范围内 |

### 真正缺失的 API 路由（5 条）
| Python 路由 | 功能 | 前端是否调用 | 影响 |
|------------|------|-------------|------|
| `/api/agent/ad-fingerprints` (GET/POST) | 广告指纹管理 | ❌ 不调用 | 低（Agent 内部工具，Rust 有 query-ad-clusters 等替代） |
| `/api/agent/ad-fingerprints/{text_hash}` (DELETE) | 删除广告指纹 | ❌ 不调用 | 低 |
| `/api/agent/tools/manifest` (GET) | Agent 工具清单 | ❌ 不调用 | 低（Agent 工具已在 Rust 实现并注册） |
| `/api/ad-clusters` (GET) | 广告聚类查询 | ❌ 不调用 | 低（Rust 有 POST /api/agent/tools/query-ad-clusters） |
| `/api/translate` (POST) | 实时翻译预览 | ❌ 不调用 | 低（广播已支持 do_translate 选项） |

**结论：5 条缺失均为服务端/Agent 内部工具接口，前端 UI 不调用，不影响 MVP 上线门禁。**

## 三、Rust 端已完整迁移的核心域（按业务域对照）

| 业务域 | Python 路由 | Rust 路由 | 状态 |
|--------|------------|----------|------|
| **认证** | /api/login, /api/login/otp, /api/login/tma, /api/csrf-token, /api/session/context | 全部同路径 | ✅ |
| **聊天** | /api/messages/{user_id}, /messages/{user_id}/ai, /api/upload/{user_id}, /api/delete_chat, /api/mark_read | 全部 + /api/search | ✅ |
| **群组** | /api/groups, /groups/config, /api/group_members*, /api/group_messages*, /api/group_photo, /api/group_verify_config, /api/group_member_count, /api/group_listen/* | 全部同路径 | ✅ |
| **广播** | /api/broadcast*, /api/push_tasks* | 全部 + /api/broadcast/unified | ✅ |
| **财务/监控** | /api/monitors*, /api/finance* | 全部 + /api/v1/monitors/finance/* 扩展 | ✅ |
| **AI** | /api/ai/chat, /api/ai/detect_ad, /api/ai/pipeline_mode | 全部 + /api/openai/* + /api/ai/skills/* | ✅ |
| **商户** | /api/merchant/* | 全部同路径 | ✅ |
| **平台** | /api/platform/* | 全部同路径 + subscriptions/downgrade | ✅ |
| **广告管道** | /api/ad-pipeline/* | 全部同路径 | ✅ |
| **MiniApp** | /api/miniapp/* | 全部同路径 | ✅ |
| **Webhook** | —（Python 内嵌） | /telegram/webhook/{bot_id} + register/dispatch-test | ✅ |
| **健康检查** | /api/health, /healthz | 全部 + /api/ready, /readyz | ✅ |

## 四、结论与建议

1. **迁移完整性**：Python 主控 115 条 API 中 **110 条已完整迁移**（95.7%），5 条缺失均为前端不调用的服务端工具接口。
2. **前端契约**：前端实际调用的 102 个 method+path 契约 **全部通过**（运行时真实请求验证，static-only=0）。
3. **不建议为 5 条缺失接口补迁移**：它们服务于 Python 内部的 Agent/n8n 工具链，前端 UI 无调用；若后续需要，可独立补齐（每个接口约 20-50 行 Rust handler）。
4. **真正的上线缺口仍是**：①人工 P-01~P-07 真机验收 ②2 条迁移跳过记录处置 ③灰度上线 ④地址监听真实链上验收——与 API 迁移无关。

---
*本报告由本地 WSL 对照 Python 源与 Rust 路由自动生成，文档位于 saas-rust/docs/rust-refactor/API-MIGRATION-AUDIT.md*
