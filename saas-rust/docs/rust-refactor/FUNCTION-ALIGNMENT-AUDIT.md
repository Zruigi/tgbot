# 功能对齐总报告（Python 源 → Rust 重构）

> 生成时间：2026-08-07 06:58（+08:00）
> 审计基准：Python 源 `/home/ubuntu/tgbot`（生产）vs Rust `/home/ubuntu/tgbot-rust-beta/saas-rust`（beta/rust-migration @ a3bd39f）
> 方法：逐项代码实测核对（非凭感觉）；所有结论均有文件/行号证据
> 总览：**API 95.7% ｜ 立项要求 13/13 ｜ Agent 工具 12/12 ｜ 租户 100%（且更强）**

---

## 一、API 层对齐（详见 API-MIGRATION-AUDIT.md）

| 维度 | 结果 |
|------|------|
| Python web_admin.py 路由 | 128 条 |
| Rust routes/mod.rs | 155 条（含 method 展开，数量更多）|
| API 迁移率 | **110/115（95.7%）** |
| 前端契约 | **102/102 全绿，static-only=0** |
| 缺失（5 条）| ad-fingerprints×2、tools/manifest、ad-clusters GET、translate —— 均为前端不调用服务端工具 |
| 页面路由 | `/` `/miniapp` `/assets/*` 由 Rust SPA fallback 覆盖（200）|

## 二、立项要求对齐（PORTING-13-COMMITS.md，详见 REQUIREMENTS-AUDIT.md）

**13/13 达成**。关键项证据：
- 要求1 快照式广告删除 → `tools/query_recent_ad_messages.rs` + `delete_recent_ad_messages.rs` + 迁移 0023
- 要求2 confirm 只报结果 → `core.rs:490`
- 要求3 虚拟助手 UID → `user_repo.rs` EXCLUDED_INBOX_USER_IDS
- 要求4 广告指纹库 → `services/ad_text.rs` + 迁移 0021（⚠️ HTTP 端点 manifest/fingerprints 未注册，前端不调用）
- 要求5-9 服务端重查/409闸/白名单路由/证据分层/limit50 → 全部 ✅
- 要求10 硬拦截+取消词表 → `deletion_guard.rs` + `safety.rs`
- 要求11-12 群ID优先/会话上下文 → `contextual_command.rs`
- 要求13 ad_governance → `services/ad_governance.rs`（1317 行）+ 迁移 0022/0025/0026

## 三、Agent 功能对齐（详见 WORKBUDDY-WORKLOG.md 行动 10）

**工具 12/12 全部迁移 + 2 个新增**：

| Python 工具类 | Rust 实现 |
|--------------|----------|
| FinanceTool | tools/finance_query.rs |
| BroadcastTool | tools/broadcast_message.rs |
| ContentGeneratorTool | tools/generate_content.rs |
| AdDetectorTool | tools/detect_advertising.rs + ad_detection.rs |
| QueryRecentAdMessagesTool | tools/query_recent_ad_messages.rs |
| DeleteRecentAdMessagesTool | tools/delete_recent_ad_messages.rs |
| DeleteAdMessageTool | agent_tools.rs::tool_delete_ad_cluster |
| QueryAdClustersTool | tools/query_ad_clusters.rs |
| DeleteAdClusterTool | agent_tools.rs（含 409 闸）|
| SystemQueryTool | tools/system_query.rs |
| TelegramSdkQueryTool | agent_tools.rs |
| SkillManageTool | tools/skill_manage.rs + /api/ai/skills/* |
| 新增（Python 无）| tools/ocr.rs、tools/n8n_workflow.rs |

核心管线：process_message / confirm_and_execute / match_tool_from_text（白名单）/ build_fallback_args / contextual_command / deletion_guard / safety 硬拦截 —— 全部对齐。

## 四、租户功能对齐（详见 WORKBUDDY-WORKLOG.md 行动 10）

**100% 对齐，且 Rust 更强**：

| 维度 | Python | Rust |
|------|--------|------|
| scope 解析 | session 一级 | **三级**：内部密钥→session→Supabase（auth.rs::resolve_scope）|
| 角色模型 | platform_admin/merchant_owner/admin/viewer | 同一套（auth.rs:42-47）|
| 数据库隔离 | 应用层过滤 | **迁移 0006/0007/0008 + RLS 0024/0026** |
| 密钥加密 | secret_manager.py | secret_crypto.rs（按租户派生）|
| 兼容 | — | legacy_merchant_id 反查 |

## 五、唯一缺口汇总（全部低影响，前端不调用）

| 缺口 | 影响 | 备注 |
|------|------|------|
| /api/agent/tools/manifest (GET) | 低 | schema/services 已实现，缺薄 handler |
| /api/agent/ad-fingerprints (GET/POST/DELETE) | 低 | 同上，迁移 0021 表已建 |
| /api/ad-clusters (GET) | 低 | Rust 已有 POST 版工具 |
| /api/translate (POST) | 低 | 广播已支持 do_translate |

## 六、结论

- **Rust 重构功能对齐度：工程实现 ≈ 90-95%**（API 95.7% + 立项 13/13 + Agent 12/12 + 租户 100%）。
- 剩余均为：①人工真机验收（P-01~P-07 + MiniApp）②2 条迁移跳过处置 ③灰度上线 ④地址监听真实链上验收 —— 与代码缺口无关。
- 如需 100% 路由对齐，可补 4 个薄 handler（合计约 100-150 行 Rust），建议上线后按需处理。

---
*本报告整合 API-MIGRATION-AUDIT.md / REQUIREMENTS-AUDIT.md / WORKBUDDY-WORKLOG.md 行动 9-10，commit a3bd39f*
