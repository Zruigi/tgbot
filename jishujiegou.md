---
alwaysApply: true
description: TG Bot 技术执行文档（精简版）
version: "6.14"
revision_date: "2026-04-03"
---

# TG Bot 技术文档 v6.14（精简执行版）

## 📋 版本与变更摘要（保留关键）

| 版本 | 日期 | 变更摘要 |
|---|---|---|
| 6.14 | 2026-04-03 | **文档精简重构**：删除重复段落与历史过程铺陈，统一为“代码引用库 + 实现链路 + 依赖矩阵 + API + DB + 迁移清单”。 |
| 6.13 | 2026-04-03 | 补齐代码引用、实现路径、依赖矩阵；开始从过程记录转向执行手册。 |
| 6.12 | 2026-04-03 | 增加按批次迁移计划与索引引用修复 SOP。 |
| 6.10 | 2026-04-03 | 明确 To-Be 分层架构与超大文件拆分路线。 |
| 6.9 | 2026-04-03 | 登录域名优先级修复：`FRONTEND_URL > WEB_BASE_URL > tunnel探测 > 127.0.0.1`。 |
| 6.8 | 2026-04-03 | 新增并稳定 OpenAI 兼容接口：`/v1/chat/completions`、`/v1/models`、`/v1/exec`。 |
| 6.7 | 2026-04-02 | 日志分层目录落地：`logs/app`、`logs/tunnel`、`logs/runtime`。 |
| 6.4 | 2026-04-02 | 统一群发结果可视化 + 管理员 Telegram 回执。 |
| 6.3 | 2026-04-01 | ChatPanel 媒体与发送状态链路修复。 |
| 6.0 | 2026-04-01 | 历史 `admin_new.html + static/js/app.js` 模块化拆分。 |

---

## 🔗 代码引用库（Code References）

### 后端主链
- 入口与配置：`main.py`、`config.py`
- 管理端 API 入口：`web_admin.py`
- Bot 事件处理：`handlers/start.py`、`handlers/group_monitor.py`、`handlers/channel_verify.py`、`handlers/reminder_cleanup.py`
- 数据访问聚合层（迁移前）：`database.py`
- 服务层：`services/translator.py`、`services/chain_monitor.py` 等

### 前端主链
- 页面：`frontend/src/views/*`
- 状态：`frontend/src/stores/*`
- API 封装：`frontend/src/api/index.ts`
- 历史页（兼容维护）：`admin_new.html`、`static/js/app.js`

---

## 🛤️ 实现路径（Execution Path）

1. **登录链路**
   - Telegram `/start`
   - `handlers/start.py` 生成 `FRONTEND_URL/?otp=`
   - 前端登录页提交 OTP
   - `web_admin.py /api/login` 建立会话

2. **聊天链路**
   - `ChatPanel + store`
   - `frontend/src/api/index.ts`
   - `web_admin.py /api/messages*`
   - `database.py`

3. **群组管理链路**
   - `GroupPanel`
   - `/api/groups*`、`/api/group_members*`、`/api/group_verify*`
   - `web_admin.py`
   - `database.py` + Bot handlers

4. **统一群发链路**
   - `BroadcastPanel`
   - `/api/broadcast/unified`
   - `web_admin.py::api_broadcast_unified`
   - Telegram Bot API + 管理员回执

5. **AI 兼容链路**
   - 客户端（Cursor/Web）
   - `/v1/chat/completions | /v1/exec | /v1/models`
   - `web_admin.py` 协议适配
   - `services/translator.py`

---

## ⚙️ 实现依赖矩阵（Implementation Dependencies）

| 层级 | 模块 | 直接依赖 | 说明 |
|---|---|---|---|
| 接入层 | `web_admin.py` | `aiohttp`, `config.py`, `database.py`, `services/*`, `handlers/*` | 当前统一入口 |
| 业务层 | `handlers/*`, `services/*` | `database.py`, Telegram Bot SDK, 第三方 API | 业务编排 |
| 数据层 | `database.py` | SQLite/SQL 执行层 | 迁移前聚合数据层 |
| 前端层 | `views/stores/api` | Vue3, Pinia, Vue Router | 管理端 UI + 状态 + 接口 |
| 基础设施 | nginx + cloudflared + aiohttp | `80 -> 8080` | `/api`、`/ws` 转发 |

---

## 🏗️ 目标架构（To-Be）

```text
tgbot/
├── backend/app/
│   ├── api/        # Router：路由/参数/响应
│   ├── services/   # Service：业务编排
│   ├── core/       # Core：配置/DB/鉴权/基础设施
│   └── schemas/
├── bot/
│   ├── handlers/
│   └── services/
├── frontend/src/
│   ├── api/
│   ├── stores/
│   └── views/
└── shared/
```

### 分层红线
- 仅允许：`api -> services -> core`
- 禁止：`core -> services/api` 反向依赖
- 单文件建议上限：Python 500 行，前端逻辑文件 350 行

---

## 🧱 大文件拆分优先级

| 优先级 | 文件 | 当前问题 | 目标 |
|---|---|---|---|
| P0 | `web_admin.py` | 路由/业务/协议混杂 | 拆至 `backend/app/api + services + core` |
| P0 | `database.py` | SQL 与数据访问全集中 | 按域拆 `*_repo.py` |
| P1 | `static/js/app.js` | 历史巨型脚本 | 收敛至 `frontend/src/stores + views` |
| P1 | `handlers/group_monitor.py` | 职责聚合过多 | handler/event 与 service 解耦 |
| P2 | `services/translator.py` | AI 网关逻辑膨胀 | provider/router/stream adapter 化 |

---

## 🌐 当前运行与转发链路

- cloudflared: `http://127.0.0.1:80`
- nginx: 监听 `80`
  - `/`、`/assets/*` -> 静态资源
  - `/api/*`、`/ws` -> `127.0.0.1:8080`
- aiohttp: 监听 `8080`
- Vite Dev: `5173`（仅开发）

---

## 🔌 API 清单（核心）

### 系统与 SPA
- `GET /`：返回 `dist/index.html`
- `GET /assets/*`：静态资源
- `GET /{非api路径}`：SPA fallback

### AI 兼容层
- `POST /v1/chat/completions`：OpenAI 兼容聊天（支持 SSE）
- `GET /v1/models`：模型列表
- `POST /v1/exec`：Exec 兼容（支持 `event: message/done`）
- `POST /api/messages/{user_id}/ai`：管理端 AI 联系人入口

### 群组与验证
- `GET /api/groups`
- `GET /api/group_members`
- `POST /api/group_members/action`
- `GET /api/group_messages`
- `POST /api/group_messages/send`
- `POST /api/group_messages/delete`
- `GET /api/group_listen/verified_users`
- `GET /api/group_stats/verify_growth`
- `GET /api/group_verify_config`
- `POST /api/group_verify_config`
- `GET /api/group_stats/verify_stats`

### 群发与任务
- `GET /api/broadcast/targets`
- `POST /api/broadcast/targets/sync`
- `POST /api/broadcast/chats`
- `POST /api/broadcast/send`
- `POST /api/broadcast/unified`
- `POST /api/push_tasks`
- `GET /api/push_tasks`
- `PATCH /api/push_tasks/status`
- `POST /api/push_tasks/run_now`

### 链上财务
- `GET /api/v1/monitors/finance/monitors`
- `POST /api/v1/monitors/finance/monitors`
- `DELETE /api/v1/monitors/finance/monitors/{id}`
- `PATCH /api/v1/monitors/finance/monitors/{id}`
- `GET /api/v1/monitors/finance/transactions`
- `GET /api/v1/monitors/finance/summary`
- `POST /api/v1/monitors/finance/reconcile`
- `GET /api/v1/monitors/finance/account_categories`
- `GET /api/v1/monitors/finance/audit_logs`
- `GET /api/v1/monitors/finance/reports/generate`

---

## 🗄️ 数据库关键表（核心字段）

### `group_members`
- `user_id`, `group_id`, `status`, `joined_at`
- `is_verified`, `verified_at`, `avatar_file_id`

### `tracked_groups`
- `delete_on_revoke`
- `verify_enabled`, `verify_type`, `verify_timeout`
- `verify_channel_id`, `verify_questions`, `verify_msg_template`

### `group_messages`
- `revoke_checked`

### `address_monitors`
- `monitor_id`, `user_id`, `address`, `network`, `remark`
- `inflow_20`, `outflow_20`, `profit_20`, `is_active`

### `finance_ledgers`
- `ledger_id`, `monitor_id`, `user_id`, `account_id`
- `direction`, `amount`, `token_symbol`, `base_fiat_value`
- `reconciled_status`, `reconciled_by`, `reconciled_at`

### `chart_of_accounts`
- `account_id`, `code`, `name`, `type`, `parent_id`, `is_active`

### `audit_logs`
- `audit_id`, `admin_userid`, `action`, `target_table`, `target_id`
- `before_state`, `after_state`, `reason`, `timestamp`

---

## ✅ 迁移执行 SOP（简版）

1. 先拆 Router 壳，再搬 Service，再收敛 Core。
2. 搬运后立即删除旧实现（Extract & Destroy），禁止双份并存。
3. 每批迁移必须保留回滚点。
4. 每批验收固定三项：
   - 接口回归（路径/参数/返回）
   - 核心流程回归（登录/聊天/群组/群发/财务）
   - 日志回归（无新增系统级异常）

---

## 🧹 归档说明

以下内容已从主文档移除（保留版本摘要，不保留重复过程细节）：
- v5.* 分阶段开发流水记录
- 同一主题的多次重复热修复展开描述
- 大段“实现过程复述”与重复约束条目

本文件仅保留：**当前可执行标准 + 可追溯索引 + 迁移规则**。
