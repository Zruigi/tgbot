# 数据库迁移指南

## 迁移顺序

```
0001_saas_core.sql        ← SaaS 核心：auth.users, merchants, plans, subscriptions
0002_merchant_public_fields.sql  ← 商户公共字段补充
0001_core.sql             ← 业务核心：chat.*, groups.*, broadcast.*
0002_auth_sessions.sql    ← 认证会话：admin_sessions, OTP, CSRF, TMA grants
0003_saas.sql             ← SaaS 扩展：potential_users, tenant_configs, oneapi keys
0004_broadcast_push.sql   ← 广播和推送任务表
0004_monitor_finance.sql  ← 监控和财务表
0005_indexes.sql          ← 热点路径补充索引
```

## Schema 问题（已修复 ✅）

以下 3 个 Schema 一致性 Bug 已在 2026-06-24 修复：

| 问题 | 文件 | 修复 |
|------|------|------|
| `merchants` 表在 public schema，`0003_saas.sql` 引用 `tenant_saas.merchants` | `0003_saas.sql` | ✅ 改为 `public.merchants` |
| `broadcast_targets` 和 `push_tasks` 在 public 和 `broadcast` schema 各一份 | `0004_broadcast_push.sql` | ✅ 添加 `broadcast.` 前缀 |
| `saas_user_roles` 在 public 和 `auth` schema 各一份 | `0001_saas_core.sql` | ✅ 改为 `auth.saas_user_roles` |

### 1. Schema 一致性
| 问题 | 文件 | 说明 |
|------|------|------|
| `merchants` 表无 schema | `0001_saas_core.sql` | 创建在 public 下，但 `0003_saas.sql` 引用 `tenant_saas.merchants` |
| `broadcast_targets` 重名 | `0001_core.sql` vs `0004_broadcast_push.sql` | 0001 创建 `broadcast.broadcast_targets`，0004 创建 `broadcast_targets`（public） |
| `push_tasks` 重名 | `0001_core.sql` vs `0004_broadcast_push.sql` | 同上 |
| `saas_user_roles` 重复 | `0001_saas_core.sql` vs `0002_auth_sessions.sql` | 0001 在 public schema，0002 在 `auth` schema |

### 2. 缺少 SCHEMA 创建
- `0001_core.sql` 使用 `chat.*`, `groups.*`, `broadcast.*` 但没有 `CREATE SCHEMA IF NOT EXISTS chat/groups/broadcast`

### 3. indexes 覆盖度
- ✅ `(tenant_id, created_at)` — 多数表已有
- ✅ `(tenant_id, group_id, timestamp desc)` — groups.group_messages 已有
- ✅ `(tenant_id, user_id)` — group_members 已有
- ⚠️ 缺少 `payment_orders(tenant_id, status)` — 需要添加
- ⚠️ 缺少 `ai_usage(tenant_id, date)` — 需要添加
- ⚠️ 缺少 `audit_logs(tenant_id, timestamp)` — finance_audit_logs 已有，但 platform/platform 的审计表可能没有

## 待添加的索引

```sql
-- 支付订单索引
CREATE INDEX IF NOT EXISTS idx_payment_orders_tenant ON payment_orders(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_status ON payment_orders(tenant_id, status);

-- AI 使用量索引
CREATE INDEX IF NOT EXISTS idx_ai_usage_tenant ON ai_usage(tenant_id);
CREATE INDEX IF NOT EXISTS idx_ai_usage_date ON ai_usage(tenant_id, date DESC);

-- 广播任务执行历史
CREATE INDEX IF NOT EXISTS idx_broadcast_results_tenant ON broadcast.broadcast_results(tenant_id);
CREATE INDEX IF NOT EXISTS idx_broadcast_results_task ON broadcast.broadcast_results(push_task_id);
```
