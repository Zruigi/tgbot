---
alwaysApply: false
description: TG Bot 项目完整规范文档 - 功能、架构、数据库、API、部署
version: "3.3"
revision_date: "2026-03-27"
---

# TG Bot 项目完整规范文档 v3.3 (2026-03-27)

## 📋 版本与变更摘要

| 版本 | 日期 | 变更内容 |
|------|------|----------|
| 3.3 | 2026-03-27 | **企业报表功能**：新增 `counterparty_aliases` 表（对方地址→人名备注，全局共享）；`_period_boundaries_ms` 支持 `year` 周期；新增 `get_year_monitor_summary()`（按月汇总全年收支）；新增 `get_counterparty_alias_map()`；Web 端新增 `GET/POST/DELETE /api/monitors/counterparty_aliases`、`GET /api/monitors/year_summary`；链上监听面板重构为地址标签页切换，交易记录自动用别名填充付款方/收款方，新增「地址备注」弹窗（全量管理）、「年账单」弹窗（月度汇总+导出）、「导出年表」Excel（含流水/月度汇总/全年合计三个 Sheet）；Excel 导出新增表头样式（深蓝底白字）、自动列宽（中文2倍宽）、对方地址备注列、地址备注列；修复所有侧边栏导航 `<a href="#">` 缺少 `.prevent` 导致点击后页面跳转的问题 |
| 3.2 | 2026-03-26 | **安全**：字段级 AES-256-GCM 加密（`services/field_crypto.py`），新增 `DB_ENCRYPT_KEY` 环境变量（32字节 base64url），加密格式 `ENC::<base64(nonce+ciphertext+tag)>`，`database.py` 全面接入 `encrypt()`/`decrypt()`（消息内容、钱包地址、备注、交易对手方）；**修复**：管理员在群聊中引用非 Bot 转发消息时静默忽略，不再回复「无法识别目标用户 ID」；**菜单 UX**：`/start` 菜单重设计，管理员与普通用户分别显示专属菜单、两列按钮布局，去除调试按钮；**管理后台入口**：`admin_dashboard` 改为 OTP 链接（5分钟有效），不再使用明文 token；**链上监听面板**：新增 `owner_type` 字段区分「管理员自有」vs「用户查询」，`get_all_monitors_for_admin` 只返回管理员自己的地址，新增 `get_user_query_addresses`/`claim_monitor_as_admin`，Web 端面板改为卡片式布局、内联交易记账编辑、「用户查询地址」独立分区支持一键转为我的监听 |
| 3.1 | 2026-03-26 | 登录身份校验强化：`/api/login` 改为两步验证码流程（Step1 发码 / Step2 校验），通过 Notify Bot 向用户 Telegram 发送 6 位随机验证码；防暴力枚举（5次上限）、60s 发码频率限制、5分钟过期、验证后立即销毁；新增 `VerifyCodeRequest` 模型、`_gen_code()`、`_send_tg_message()`、`/api/login/verify` 路由；前端登录表单改为双步视图（step1/step2），支持「重新发送」；主控平台新增 `NOTIFY_BOT_TOKEN` 配置项 |
| 3.0 | 2026-03-26 | 云端多租户部署方案：新增主控平台 `tgbot-platform/`，支持用户自助注册并输入 Bot Token 启动独立实例；USDT TRC20 金额指纹自动收款（每人小数唯一，自动匹配链上转账）；订阅到期自动掐断容器授权；用户门户前端（注册/登录/控制台/续费/套餐页）；修复 `config.py` 中 `DB_NAME` 支持 `os.getenv("DB_NAME")` 覆盖以实现租户数据隔离 |
| 2.5 | 2026-03-25 | 手机端全分辨率响应式适配：左侧 sidebar 手机端隐藏，新增底部固定 Tab Bar（5个tab：会话/群组/群发/潜在/链上），支持 `env(safe-area-inset-bottom)` 安全区适配；聊天/群组工作台实现「列表→详情」双视图切换（`mobilePanelView`），详情页顶部新增返回按钮；群成员弹窗从 `max-w-2xl` 扩大到 `max-w-4xl`，成员列表从2列改为3列，工具栏固定不被撑大；群成员列表新增无限下滑加载（IntersectionObserver，初始30条，每次+20条），搜索变化自动重置；新增直接输入UID操作区（禁言/解禁/移除），不需要成员在列表中；群实际人数通过 `getChatMemberCount` API 获取显示 |
| 2.4 | 2026-03-25 | 修复群聊工作台点击群组无响应（`selectGroupRoom` 方法缺失）；修复群消息 API 报错 `no such column: avatar_file_id`（`group_members` 表缺列，已补 ALTER TABLE）；修复 Vue 3 中 `this.$set` 调用报错（替换为展开赋值）；修复群聊工作台头像不显示（`selectGroupRoom` 新增静默预加载成员头像逻辑）；更新数据库依赖路径与排查指南 |
| 2.3 | 2026-03-25 | 修复 Vue 2 响应式头像不显示（`$set` 替换直接赋值）；修复用户列表头像 v-if 条件；群消息对账循环优化（48h 窗口、并发 3 群、30s 间隔、限速保护）；群组监听新增 `get_chat()` 头像缓存采集，写入 `admin_inbox` 同步私聊头像；`autoLoadFile` 与 `loadFile` 统一改用 `$set`；修复群消息时间戳显示错误（兼容 `message_ts` Unix 时间戳与 `timestamp` 字符串两种字段）；`formatTime()` 修复 UTC+8 时区自动补正；群消息图片/媒体不显示修复（`refreshGroupRoomMessages` 批量 `autoLoadFile`） |
| 2.2 | 2026-03-24 | 修复 Web 管理端群发"必须先保存草稿才能发送"的限制：支持不保存草稿直接群发；同步更新按钮状态校验与发送逻辑，保留草稿发送能力 |
| 2.1 | 2026-03-24 | 优化 TG 端回复文案：减少表情、统一简洁格式；新增每条关键消息固定包含用户名与 UserID；补充媒体消息映射，提升机器人识别与引用回复成功率；更新规范文档与路径引用 |
| 2.0 | 2026-03-23 | 合并所有 *.emd、skill *.md 内容；补充数据库完整架构、API 路由、Web 前端功能清单；修复贴纸渲染、30分钟超时、Web UI 优化 |

---

## 📑 目录

1. [项目全景图](#项目全景图)
2. [核心功能模块](#核心功能模块)
3. [数据库架构](#数据库架构)
4. [Web 管理端](#web-管理端)
5. [API 接口清单](#api-接口清单)
6. [部署与运维](#部署与运维)
7. [已知问题与修复](#已知问题与修复)

---

## 项目全景图

### 系统架构

```
┌─────────────────────────────────────────────────────────────┐
│                    Telegram Bot (main.py)                   │
│  ├─ handlers/          (业务逻辑处理器)                      │
│  ├─ services/          (后端核心服务)                        │
│  ├─ database.py        (异步数据库层)                        │
│  ├─ config.py          (全局配置)                            │
│  └─ web_admin.py       (Web 管理端后台)                      │
└─────────────────────────────────────────────────────────────┘
         ↓                                    ↓
    Telegram API                    Web 管理端 (admin_new.html)
  (polling/webhook)                 (Vue 3 + Tailwind CSS)
         ↓                                    ↓
    SQLite DB                        WebSocket + REST API
  (crypto_bot.sqlite3)               (127.0.0.1:8080)
```

### 核心依赖

- **python-telegram-bot** (20.0+): Telegram Bot API 异步框架
- **aiohttp**: Web 服务器与异步 HTTP 客户端
- **aiosqlite**: 异步 SQLite 驱动
- **ccxt**: 币安行情查询（免鉴权公开 API）
- **httpx**: 链上 API 调用（TronGrid、BscScan）
- **deep-translator**: 翻译服务（Google Translate 备选）
- **langdetect**: 自动语言检测
- **pytesseract + PIL**: OCR 地址核验
- **psutil**: 系统监控

---

## 核心功能模块

### 1. 客服系统 (handlers/customer_service.py)

**功能**：双向消息转发、头像缓存、快速回复、一键群发、OCR 核验

**关键流程**：
- 用户消息 → 自动转发给管理员 (Telegram 私聊)
- 管理员回复 → 自动转发给用户 + Web 端同步
- 待发送消息队列 (pending_admin_messages) 支持 30 分钟自动失效
- 全媒体支持：文本、图片、视频、音频、贴纸、动画、文件

**头像获取逻辑**：
- 每次用户发消息时调用 `get_user_profile_photos(user_id, limit=1)`
- 获取到的 `file_id` 存入 `admin_inbox.avatar_file_id`
- 受 Telegram 隐私设置限制（用户设为"所有人"才可见）

**2026 修复**：
- 贴纸渲染：支持 `.webm` 视频贴纸在 Web 端播放
- 30分钟超时：修复 UTC 时区误判导致全部消息过期的 bug
- 占位文本过滤：发送贴纸时不再显示 `[sticker]` 文本

### 2. 群组监听 (handlers/group_monitor.py)

**功能**：自动捕捉潜在用户、关键词检测、画像标签映射

**关键特性**：
- 8 个默认关键词 + 自定义关键词支持
- 画像标签引擎：自动分类用户意向（拉新、合作、投融资等）
- 标签累积逻辑：同一用户多次触发不同关键词时自动去重合并
- 群组配置缓存：优化性能，支持 Web 端实时切换监听状态
- WebSocket 实时推送：群组活跃状态秒级更新

**头像采集（2026-03-25 新增）**：
- 模块顶部维护 `_avatar_cache: dict`（内存 KV，user_id → file_id）
- 每条群消息到达时检查缓存，未命中则调用 `context.bot.get_chat(user_id)` 获取 `chat.photo.small_file_id`
- 结果写入缓存，**每用户只调一次 API**，避免限速
- 同步写入 `group_messages.avatar_file_id` 与 `admin_inbox.avatar_file_id`
- Telegram 隐私设置为"所有人"可见时才能获取到头像

### 3. 链上监听 (services/chain_monitor.py)

**功能**：TRC20 (TRON) 与 BEP20 (BSC) USDT 钱包实时监听

**关键特性**：
- 20 秒轮询间隔（严格遵守）
- 交易防重机制：last_tx_timestamp 避免重复推送
- 精度换算：TRC20 ÷10^6，BEP20 ÷10^18
- 风险识别：TRC20 能量租赁地址检测
- 实时通知：用户 + Web 端 + 交易哈希链接

**数据来源**：
- TronGrid API (TRON-PRO-API-KEY)
- BscScan API (BSCSCAN_API_KEY)

### 4. 币安行情监控 (services/price_monitor.py)

**功能**：实时价格通道监控与自动报警

**状态机**：
- waiting_cross_upper → 等待价格上穿上行线
- crossed_upper_waiting_lower → 已上穿，等待下穿下行线
- triggered → 触发报警，自动关闭任务

**轮询间隔**：10 秒（严格遵守）

### 5. 信息校对对账 (main.py → revoke_sync_loop)

**功能**：定时检测群消息是否被撤回，同步删除 Web 端记录

**2026-03-25 优化**：

| 项目 | 之前 | 现在 |
|------|------|------|
| 检查间隔 | 60s | 30s |
| 每群消息范围 | 最近 100 条（无时间限制） | 最近 48h 内，最多 200 条 |
| 多群处理方式 | 串行，一个接一个 | 并发，最多同时 3 群 |
| 限速保护 | 无 | 每 5 条暂停 0.3s，触发 flood 自动等 5s |
| WebSocket 通知 | 每条单独推送 | 批量统一推送 |
| 超过 48h 的消息 | 一直重复检查 | 自动跳过 |

**`get_groups_with_revoke_sync()` 改动**：
- SQL 加 `COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= cutoff_ts` 过滤 48h 外消息
- LIMIT 从 100 提升到 200

### 6. Web 管理端 (web_admin.py + admin_new.html)

**功能**：实时消息处理、链上监听管理、潜在用户筛选、群组监控

**前端标签页**：
- 会话列表 (chat)：用户列表 + 聊天窗口
- 群发功能 (broadcast)：一键向多用户发送消息
- 链上监听 (onchain_monitors)：CRUD 管理钱包监听
- 潜在用户 (potential_users)：分页、标签过滤、一键转正
- 群组管理 (group_management)：监听开关、自定义关键词、消息历史

**认证机制**：
- Bearer Token 中间件 (WEB_ADMIN_TOKEN)
- 本地回环 (127.0.0.1) 自动免登
- Cookie 持久化 (admin_token)

**状态持久化**：
- localStorage: activeTab、selectedUserId、selectedUserName
- 页面刷新后自动恢复

---

## 数据库架构

### 数据库实例

| 文件 | 状态 | 说明 |
|------|------|------|
| `crypto_bot.sqlite3` | 活跃 | 主数据库，包含 20 个表（v3.3 新增 counterparty_aliases）|
| `bot.db` | 废弃 | 空表，已停用 |

### 表结构与字段

#### 1. messages (消息表)

| 字段 | 类型 | 说明 |
|------|------|------|
| msg_id | INTEGER PK | 消息 ID（自增） |
| from_userid | INTEGER | 发送者 ID |
| to_userid | INTEGER | 接收者 ID |
| content | TEXT | 消息内容 |
| type | TEXT | 消息类型 (text/photo/video/...) |
| file_id | TEXT | 媒体文件 ID |
| timestamp | DATETIME | 消息时间 |
| is_read | BOOLEAN | 已读标记 |
| status | TEXT | 发送状态 (sent/sending/failed) |
| translated_text | TEXT | 翻译后文本 |
| original_text | TEXT | 原始文本 |
| admin_userid | TEXT | 管理员 ID |

#### 2. admin_inbox (收件箱汇总)

| 字段 | 类型 | 说明 |
|------|------|------|
| from_userid | INTEGER PK | 用户 ID |
| from_username | TEXT | 用户名 |
| summary | TEXT | 最后一条消息摘要 |
| unread_count | INTEGER | 未读数 |
| last_active | DATETIME | 最后活跃时间 |
| avatar_file_id | TEXT | 用户头像 file_id（2026 新增） |

**头像写入时机**：
1. 用户私聊发消息 → `get_user_profile_photos()` → 写入
2. 用户在被监听群里发消息 → `get_chat()` → 写入（2026-03-25 新增）
3. 受 Telegram 隐私设置限制，头像不公开的用户永远为 NULL

#### 3. group_messages (群消息)

| 字段 | 类型 | 说明 |
|------|------|------|
| msg_id | INTEGER PK | 消息 ID |
| group_id | INTEGER | 群组 ID |
| user_id | INTEGER | 发送者 ID |
| username | TEXT | 发送者用户名 |
| content | TEXT | 消息内容 |
| msg_type | TEXT | 消息类型 |
| file_id | TEXT | 媒体 ID |
| tg_msg_id | INTEGER | Telegram 消息 ID |
| sender_type | TEXT | 发送者类型 |
| quoted_tg_msg_id | INTEGER | 引用消息 ID |
| quoted_text | TEXT | 引用内容 |
| message_ts | INTEGER | Unix 时间戳 |
| avatar_file_id | TEXT | 发送者头像 file_id |

#### 4. tracked_groups (群组追踪)

| 字段 | 类型 | 说明 |
|------|------|------|
| group_id | INTEGER PK | 群组 ID |
| group_title | TEXT | 群组名称 |
| is_monitored | BOOLEAN | 是否监听 |
| custom_keywords | TEXT | 自定义关键词 |
| delete_on_revoke | BOOLEAN | 是否开启信息校对 |
| first_seen | DATETIME | 首次发现 |
| updated_at | DATETIME | 最后更新 |

#### 5. potential_users (潜在用户)

| 字段 | 类型 | 说明 |
|------|------|------|
| user_id | INTEGER PK | 用户 ID |
| username | TEXT | 用户名 |
| message_text | TEXT | 触发消息 |
| keywords | TEXT | 匹配关键词 |
| profile_tags | TEXT | 画像标签 |
| group_id | INTEGER | 来源群组 |
| captured_at | DATETIME | 捕捉时间 |

#### 6. group_members (群成员)

| 字段 | 类型 | 说明 |
|------|------|------|
| id | INTEGER PK | 自增 ID |
| user_id | INTEGER | 用户 ID |
| username | TEXT | 用户名 |
| first_name | TEXT | 名 |
| last_name | TEXT | 姓 |
| group_id | INTEGER | 群组 ID |
| group_title | TEXT | 群组名称 |
| status | TEXT | 状态 (active/left/admin) |
| joined_at | DATETIME | 加入时间 |
| avatar_file_id | TEXT | 头像 file_id（v2.4 补加，原迁移漏掉） |

> **⚠️ 迁移注意**：`avatar_file_id` 列在 v2.4 之前未被 `init_db()` 自动添加到 `group_members` 表。若遇到 `/api/group_messages` 报 `no such column: avatar_file_id`，需手动执行：
> ```sql
> ALTER TABLE group_members ADD COLUMN avatar_file_id TEXT DEFAULT NULL;
> ```

#### 7. address_monitors (钱包监听)

| 字段 | 类型 | 说明 |
|------|------|------|
| monitor_id | INTEGER PK | 监听 ID |
| user_id | INTEGER | 用户 ID |
| address | TEXT | 钱包地址（AES-256-GCM 加密）|
| network | TEXT | 网络 (TRC20/BSC) |
| last_tx_timestamp | INTEGER | 最后交易时间 |
| is_active | BOOLEAN | 是否活跃 |
| remark | TEXT | 备注（AES-256-GCM 加密）|
| risk_level | TEXT | 风险等级 |
| risk_note | TEXT | 风险说明 |
| created_at | DATETIME | 创建时间 |
| owner_type | TEXT | 所有权类型：`admin_own`（管理员自有）/ `user`（用户查询，v3.2 新增）|

#### 7. monitor_transactions (交易记录)

| 字段 | 类型 | 说明 |
|------|------|------|
| tx_id | TEXT PK | 交易哈希 |
| monitor_id | INTEGER | 监听 ID |
| user_id | INTEGER | 用户 ID |
| address | TEXT | 钱包地址 |
| direction | TEXT | 方向 (in/out) |
| amount | REAL | 金额 |
| token_symbol | TEXT | 代币符号 |
| tx_timestamp | INTEGER | 交易时间（ms） |
| counterparty | TEXT | 对方地址（AES-256-GCM 加密）|
| risk_level | TEXT | 风险等级 |
| risk_note | TEXT | 风险说明 |
| income_reason | TEXT | 收入原因（v3.2 新增）|
| expense_reason | TEXT | 支出原因（v3.2 新增）|
| payer | TEXT | 付款方（v3.2 新增）|
| payee | TEXT | 收款方（v3.2 新增）|

#### 8. counterparty_aliases (对方地址备注，v3.3 新增)

| 字段 | 类型 | 说明 |
|------|------|------|
| alias_id | INTEGER PK | 自增 ID |
| address | TEXT UNIQUE | 对方地址（链上） |
| name | TEXT | 备注人名 |
| note | TEXT | 说明备注（可选）|
| created_at | DATETIME | 创建时间 |

**用途**：全局共享的地址→人名映射表。导出 Excel 时自动填充「对方备注」列；展开交易记录时自动将已知地址补入付款方/收款方字段，无需每笔手动填写。

### 索引策略

```sql
CREATE INDEX idx_group_msg_query ON group_messages(group_id, timestamp DESC);
CREATE INDEX idx_potential_tags ON potential_users(profile_tags);
CREATE INDEX idx_potential_group ON potential_users(group_id);
CREATE INDEX idx_tracked_updated ON tracked_groups(updated_at);
```

---

## Web 管理端

### 前端架构

- **框架**：Vue 3 (Composition API)
- **样式**：Tailwind CSS + 自定义 Material Design 配色
- **实时通信**：WebSocket (ws://127.0.0.1:8080/ws)
- **图片查看**：Viewer.js 库
- **字体**：Manrope (Google Fonts)

### 响应式布局（手机端适配，v2.5 新增）

手机端（< 768px `md` 断点）采用**单栏双视图**模式：

**底部 Tab Bar**：
- 替代左侧 sidebar，固定在屏幕底部
- 5 个 tab：会话 / 群组 / 群发 / 潜在用户 / 链上监听
- 当前 tab 图标变蓝 + `FILL 1` 填充效果
- 支持 `env(safe-area-inset-bottom)` 适配 iPhone 刘海屏
- main 区域底部留 `pb-16` 防遮挡

**双视图切换（`mobilePanelView`）**：
- `'list'`：显示侧边栏列表，隐藏详情区
- `'detail'`：隐藏列表，全屏显示详情区
- 点击用户/群组卡片 → 自动切换到 `'detail'`
- 详情区顶部 header 新增 `←` 返回按钮（`md:hidden`）
- 切换 tab 时自动重置为 `'list'`

**涉及的 data 状态**：
```js
mobilePanelView: 'list', // 'list' | 'detail'
```

**涉及的 watch**：
```js
activeTab(newTab) {
    this.mobilePanelView = 'list'; // 切换 tab 时重置
}
```

**桌面端**：`hidden md:flex` / `flex md:flex` 保证桌面端行为完全不变。

### 响应式注意事项（Vue 3）

> **重要**：项目使用 **Vue 3**（CDN `unpkg.com/vue@3`），**不存在 `this.$set`**，调用会直接报 `TypeError`。

```js
// 错误写法（Vue 3 中 $set 不存在，运行时抛异常）
this.$set(this.fileUrls, fileId, url)

// 正确写法（展开赋值，触发 Vue 3 响应式）
this.fileUrls = { ...this.fileUrls, [fileId]: url }
this.fileErrors = { ...this.fileErrors, [fileId]: true }
```

**受影响函数**：`autoLoadFile()`、`loadFile()`（已于 2026-03-25 v2.4 修复）

### 功能标签页

#### 1. 会话列表 (chat)

**功能**：
- 用户列表（左侧）：搜索、未读计数、头像展示
- 聊天窗口（右侧）：消息历史、实时轮询、智能滚动
- 消息气泡：微信风格、自适应宽度、媒体支持
- 快速回复：模板按钮、一键群发、引用回复

**头像显示条件**（2026-03-25 修复）：
```html
<!-- 必须同时满足 avatar_file_id 存在 AND fileUrls 已加载 -->
<div v-if="user.avatar_file_id && fileUrls[user.avatar_file_id]"><img .../></div>
<div v-else>首字缩写</div>
```

**接口调用**：
- GET /api/users → 用户列表
- GET /api/messages/{user_id} → 消息历史
- POST /api/messages/{user_id} → 发送消息
- POST /api/mark_read/{user_id} → 标记已读

#### 2. 群发功能 (broadcast)

**功能**：
- 选择目标用户（多选）
- 编写消息内容 + 图片上传
- 草稿保存与草稿列表选择
- 支持不保存草稿直接群发（2026-03-24 修复）
- 实时进度条、成功/失败统计

#### 3. 链上监听 (onchain_monitors)

**功能**：监听地址 CRUD、近 20 笔交易统计、风险等级标记、备注编辑

#### 4. 潜在用户 (potential_users)

**功能**：分页加载（12 条/页）、画像标签过滤、关键词搜索、一键转正

#### 5. 群组管理 (group_management)

**功能**：群组列表、实时在线状态、监听开关、自定义关键词、群成员管理、消息历史

**群成员弹窗（v2.5 优化）**：
- 弹窗宽度扩大至 `max-w-4xl`（896px），高度 `max-h-[90vh]`
- 成员列表 3 列网格（桌面端），充分利用宽度
- 工具栏 `flex-shrink-0` 固定高度，成员列表 `flex-1 min-h-0` 正确滚动
- 无限下滑加载：初始渲染 30 条（`memberVisibleCount`），滚动到底自动 +20 条
  - 使用 `IntersectionObserver` 观察哨兵元素（`ref="memberScrollSentinel"`）
  - 搜索词变化自动重置回 30 条并重新绑定 Observer
  - 弹窗关闭时自动 `disconnect` Observer，防止内存泄漏
- 直接输入 UID 操作区：无需成员在列表中，直接输入 UID 执行禁言/解禁/移除
  - 调用 `POST /api/group_members/action`
  - 操作结果 3 秒后自动消失
- 群实际总人数通过 `GET /api/group_member_count?group_id=xxx` 获取并显示

---

## API 接口清单

### 认证

```
Authorization: Bearer {WEB_ADMIN_TOKEN}
或
Cookie: admin_token={WEB_ADMIN_TOKEN}
```

### 消息接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET | /api/users | 获取用户列表（含 avatar_file_id） |
| GET | /api/messages/{user_id} | 获取聊天历史 |
| POST | /api/messages/{user_id} | 发送消息 |
| POST | /api/broadcast | 群发消息 |
| POST | /api/mark_read/{user_id} | 标记已读 |
| POST | /api/delete_chat/{user_id} | 软删除会话 |

### 链上监听接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET | /api/monitors | 列表（仅管理员自有地址）|
| POST | /api/monitors | 新增 |
| DELETE | /api/monitors/{id} | 删除 |
| POST | /api/monitors/{id}/remark | 更新备注 |
| GET | /api/monitors/pnl_summary | 盈亏汇总（day/week/month）|
| GET | /api/monitors/export | 导出 Excel（day/week/month/year，自动列宽，别名填充）|
| GET | /api/monitors/year_summary | 年度月度盈亏汇总（v3.3 新增）|
| GET | /api/monitors/counterparty_aliases | 获取所有对方地址备注（v3.3 新增）|
| POST | /api/monitors/counterparty_aliases | 新增/更新对方地址备注（v3.3 新增）|
| DELETE | /api/monitors/counterparty_aliases/{id} | 删除对方地址备注（v3.3 新增）|
| POST | /api/monitors/tx_meta | 更新单笔交易业务字段 |
| POST | /api/monitors/tx_meta/batch | 批量更新交易业务字段 |
| GET | /api/monitors/user_queries | 用户查询地址列表（v3.2 新增）|
| POST | /api/monitors/{id}/claim | 将用户查询地址转为管理员自有（v3.2 新增）|

### 潜在用户接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET | /api/potential_users | 列表（支持分页、过滤） |
| GET | /api/potential_users/tags | 标签列表 |
| POST | /api/potential_users/{id}/promote | 转正 |

### 群组接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET | /api/group_listen/list | 群组列表 |
| GET | /api/group_listen/stats | 监测面板统计 |
| POST | /api/group_listen/toggle | 切换监听 |
| GET | /api/group_members | 群成员 |
| GET | /api/group_messages | 消息历史 |
| POST | /api/group_messages/delete | 删除消息 |

### 其他接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET | /api/file/{file_id} | 获取媒体 URL（代理 Telegram） |
| POST | /api/translate | 实时翻译预览 |
| POST | /api/upload/{user_id} | 图片上传 |
| GET | /api/config | 获取配置 |
| POST | /api/config | 保存配置 |

---

## 云端多租户部署（v3.0 新增）

### 架构总览

```
┌─────────────────────────────────────────────────────────────┐
│                主控平台 (tgbot-platform/)                    │
│                                                             │
│  用户门户 (index.html)  →  主控 API (control/main.py)        │
│  注册/登录/控制台/续费      FastAPI + JWT 认证               │
│                              │                              │
│  ┌───────────────────────────▼──────────────────────────┐   │
│  │          主控数据库 (control/control.db)              │   │
│  │  tenants: tg_id, bot_token, plan, expires_at, port   │   │
│  │  payments: expected_amount, fingerprint, status      │   │
│  │  port_pool: 8100-8199 空闲端口管理                   │   │
│  └──────────────────────────────────────────────────────┘   │
│                              │                              │
│  USDT 链上监测               │  订阅到期调度器              │
│  payment_monitor.py          │  scheduler.py                │
│  每 30s 扫 TronGrid          │  每 1h 检查 expires_at       │
└──────────────────────────────┼──────────────────────────────┘
                               │ 启动/停止/销毁
               ┌───────────────┼───────────────┐
               ▼               ▼               ▼
         租户实例 A       租户实例 B       租户实例 C
      (现有 bot 代码)  (现有 bot 代码)  (现有 bot 代码)
      .env: TOKEN_A    .env: TOKEN_B    .env: TOKEN_C
      DB: tenants/A/   DB: tenants/B/   DB: tenants/C/
      Port: 8101       Port: 8102       Port: 8103
```

### 目录结构

```
tgbot-platform/
├── control/
│   ├── main.py             # FastAPI 主入口（注册/登录/订单/管理员接口）
│   ├── database.py         # 主控数据库操作
│   ├── fingerprint.py      # 金额指纹分配与匹配
│   ├── payment_monitor.py  # USDT TRC20 链上收款监测
│   ├── scheduler.py        # 订阅到期检查与自动掐断
│   ├── tenant_manager.py   # 租户进程/容器管理
│   └── templates/
│       └── index.html      # 用户门户前端（深色风格）
├── tenants/                # 运行时自动生成（各租户数据目录）
│   └── <tg_id>/
│       ├── .env            # 租户专属环境变量
│       └── bot.db          # 租户专属数据库
├── Dockerfile              # Bot 实例镜像
├── Dockerfile.control      # 主控平台镜像
├── docker-compose.yml      # 编排文件
└── .env.example            # 配置模板
```

### 金额指纹机制

每个用户下单时自动分配唯一小数尾数，用于链上自动识别收款归属：

```
套餐基础价 + 指纹小数 = 用户专属付款金额

月付 15 USDT：
  用户A → 15.0137 USDT
  用户B → 15.0284 USDT
  用户C → 15.0591 USDT

监测到链上转账 15.0137 → 精确匹配用户A → 自动激活订阅
```

- 指纹范围：`0.0010 ~ 0.0990`，步进 `0.0001`，共 890 个 slot
- 匹配容差：`±0.00005`（防止链上精度损失误判）
- 订单有效期：24 小时，到期 slot 自动释放复用
- 核心模块：`control/fingerprint.py`

### 订阅生命周期

```
注册 → trial(7天) → 到期前3天发提醒 → 到期暂停容器
                                          ↓
                               续费(USDT自动识别)
                                          ↓
                               自动重启容器 + 延长 expires_at
                                          ↓
                        到期超30天(可选) → 销毁数据
```

### 主控平台 API 接口

| 方法 | 路由 | 说明 |
|------|------|------|
| GET  | /                        | 用户门户首页 |
| POST | /api/register            | 注册（验证 Bot Token + 激活试用） |
| POST | /api/login               | 登录（返回 JWT Cookie） |
| GET  | /api/me                  | 当前用户信息 + 实例状态 |
| GET  | /api/plans               | 套餐列表 |
| POST | /api/order               | 创建付款订单（返回金额指纹） |
| GET  | /api/order/status        | 轮询订单支付状态 |
| GET  | /api/admin/tenants       | 所有租户列表（管理员） |
| POST | /api/admin/instance/start| 手动启动实例（管理员） |
| POST | /api/admin/instance/stop | 手动停止实例（管理员） |

### 主控平台配置（.env）

| 变量 | 说明 | 必填 |
|------|------|------|
| `RECEIVE_USDT_ADDRESS` | 你的 USDT TRC20 收款地址 | ✅ |
| `TRONGRID_API_KEY`     | TronGrid API Key（链上监测） | ✅ |
| `NOTIFY_BOT_TOKEN`     | 用于给用户发通知的 Bot Token | ✅ |
| `JWT_SECRET`           | JWT 签名密钥（随机长字符串） | ✅ |
| `ADMIN_TG_IDS`         | 管理员 TG ID（逗号分隔） | ✅ |
| `PLATFORM_SECRET`      | 平台内部密钥（生成各租户 Web Token） | ✅ |
| `LAUNCH_MODE`          | `process`（本机）或 `docker`（容器） | 默认 process |
| `BOT_SRC_DIR`          | Bot 源码目录绝对路径 | ✅ |
| `TENANTS_DIR`          | 租户数据存储目录 | ✅ |
| `AUTO_DESTROY_STALE`   | 到期 30 天后自动销毁数据 | 默认 false |

### 本地启动（进程模式，无需 Docker）

```bash
cd d:\开发\tgbot-platform
copy .env.example .env
# 编辑 .env 填入真实值

pip install fastapi "uvicorn[standard]" aiosqlite httpx pyjwt jinja2 python-dotenv
cd control
python main.py
# 访问 http://127.0.0.1:8000
```

### 登录身份校验（v3.1 两步验证码）

登录不再仅凭 `tg_id` 即可通过，改为 Telegram OTP 两步流程：

```
Step 1  POST /api/login          → 校验账户存在，生成 6 位验证码，通过 Notify Bot 发至用户 Telegram
Step 2  POST /api/login/verify   → 用户提交验证码，校验通过后签发 JWT Cookie
```

**安全防护：**

| 机制 | 参数 |
|------|------|
| 验证码有效期 | 5 分钟 |
| 最大尝试次数 | 5 次（超出即作废） |
| 发码频率限制 | 60 秒内只能发一次 |
| 验证码生命周期 | 验证通过立即销毁（一次性） |
| 身份绑定 | 验证码仅发至用户自己的 Telegram，他人无法冒用 |

**新增代码（`control/main.py`）：**
- `import random, string`
- `NOTIFY_BOT_TOKEN = os.getenv("NOTIFY_BOT_TOKEN", "")`
- `_login_codes: dict = {}` — 内存 OTP 存储
- `_gen_code()` — 生成 6 位大写字母+数字验证码
- `_send_tg_message(tg_id, text)` — 通过 Notify Bot 发送 TG 消息
- `VerifyCodeRequest` — Pydantic 模型（tg_id + code）
- `POST /api/login/verify` — 验证码校验路由

**前端（`control/templates/index.html`）：**
- 登录面板拆分为 `#login-step1`（输入 tg_id）和 `#login-step2`（输入验证码）
- `doSendCode()` — 发码，切换到 step2
- `doVerifyCode()` — 提交验证码，成功后跳转控制台
- `resetLoginStep()` — 重新发送，回到 step1

**使用前提：** 用户须先在 Telegram 向 Notify Bot 发送 `/start`，Bot 才能主动向其发消息。

**`.env` 新增必填项：**
```
NOTIFY_BOT_TOKEN=your_notify_bot_token
```

### 无域名外网访问

```bash
# 方案1：直接 IP（VPS 部署后）
http://服务器IP:8000

# 方案2：Cloudflare Tunnel 免费隧道（本地测试）
cloudflared tunnel --url http://localhost:8000
# 自动生成 https://xxx.trycloudflare.com
```

### config.py 修改说明（v3.0）

```python
# 修改前（写死路径）
DB_NAME = os.path.join(BASE_DIR, "crypto_bot.sqlite3")

# 修改后（支持环境变量覆盖）
DB_NAME = os.getenv("DB_NAME") or os.path.join(BASE_DIR, "crypto_bot.sqlite3")
```

主控平台启动租户实例时，自动在 `.env` 中注入 `DB_NAME=/tenants/<tg_id>/bot.db`，实现每个租户数据库完全隔离，本地开发行为不变。

---

## 部署与运维

### 启动检查

运行 `startup_check.py`：
- Python 版本检查 (3.8+)
- 必要文件检查
- 依赖包检查
- 配置有效性检查

### 重启管理

**Windows 批处理** (`restart_bot.bat`)：
- 快速重启：直接杀死并启动
- 安全重启：含启动检查
- 诊断系统：性能监控
- 手动启动：交互式启动

**VBS 脚本** (`一键重启机器人.vbs`)：
- 静默启动（无窗口）
- 路径自适应

### Nginx 配置

```nginx
location /ws {
    proxy_pass http://127.0.0.1:8080/ws;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
}

location /api/ {
    proxy_pass http://127.0.0.1:8080/api/;
    proxy_set_header X-Real-IP $remote_addr;
}
```

### 数据库迁移

自动迁移脚本在 `database.py` 中执行：
- 表结构创建
- 列添加（ALTER TABLE）
- 索引创建
- 默认设置初始化

---

## 已知问题与修复

### 2026-03-27 新增/修复（v3.3）

#### 1. 企业报表功能全面升级

**新增数据库表** `counterparty_aliases`：
- 全局共享的链上地址→人名备注映射
- `UNIQUE(address)` 约束，同一地址只有一条记录（upsert 语义）
- 新增函数：`get_counterparty_aliases()`、`upsert_counterparty_alias()`、`delete_counterparty_alias()`、`get_counterparty_alias_map()`

**`_period_boundaries_ms` 新增 `year` 周期**：
- `year`：当年 1 月 1 日 00:00 → 次年 1 月 1 日 00:00
- `previous=True` 时返回上一年范围

**新增 `get_year_monitor_summary()`**：
- 按月汇总全年收入/支出/净利润/交易笔数，返回固定 12 行
- 无数据的月份自动补 0，前端无需额外处理

**Excel 导出全面重构**（`api_export_monitor_transactions`）：
- 支持 `period=year`，自动生成「链上流水」+「汇总」+「{year}年月度汇总」三个 Sheet
- 表头深蓝底（`#1E3A5F`）白字加粗居中
- 新增「地址备注」「对方备注」两列，自动从 `addr_remark_map` / `alias_map` 填充
- 付款方/收款方若用户未填则自动用别名补全（`direction=in` 填付款方，`direction=out` 填收款方）
- 所有 Sheet 自动列宽：中文字符按 2 倍宽计算，最小 8，最大 60

**前端链上监听面板重构**：
- 地址列表改为**标签页切换**：每个监听地址是一个标签按钮，点击切换，内嵌勾选框同步统计范围
- 交易记录展开时自动用 `counterpartyAliases` 补填付款方/收款方
- 交易行新增 `👤` 按钮（`person_add`）快速为对方地址命名
- 新增「地址备注」按钮 → 弹出全量管理面板（新增/删除）
- 新增「年账单」按钮 → 弹出年度月度汇总弹窗（支持切换年份、查询、导出年表）
- 导出工具栏新增「导出年表」按钮

**新增 JS 方法**：
- `loadCounterpartyAliases()` — 加载所有别名
- `saveCounterpartyAlias(address)` — 快速为地址命名（prompt）
- `saveCounterpartyAliasDirect()` — 弹窗表单直接保存
- `deleteCounterpartyAlias(aliasId)` — 删除别名
- `loadYearSummary()` — 加载年度月度汇总
- `loadMonitors()` 改造 — 加载完成后默认选中第一个地址标签，同时加载别名
- `loadMonitorTxs(m)` 改造 — 展开交易时自动用别名填充付款方/收款方

#### 2. 侧边栏导航 `href="#"` 缺少 `.prevent` 修复

**问题**：所有桌面端侧边栏 `<a href="#" @click="activeTab = 'xxx'">` 点击时浏览器触发默认跳转行为，Vue 的 `@click` 虽然执行了 `activeTab` 切换，但随即页面滚动到顶部或触发 hashchange，导致 Tab 切换看起来无效或功能入口「不存在」。

**受影响的 Tab**：`chat`、`broadcast`、`onchain_monitors`、`potential_users`、`group_management`

**修复**：所有桌面端侧边栏导航按钮的 `@click` 改为 `@click.prevent`，移动端底部 Tab Bar 已使用 `@click.prevent` 不受影响。

**代码位置**：`admin_new.html` 侧边栏导航区域（第 254、258、260 行附近）

---

### 2026-03-26 新增/修复（v3.2）

#### 1. 字段级 AES-256-GCM 加密

**新增模块**：`services/field_crypto.py`

- 密钥从环境变量 `DB_ENCRYPT_KEY` 读取（32字节 base64url 编码，无填充，44字符）
- 加密格式：`ENC::<base64url(nonce[12] + ciphertext + tag[16])>`
- 未加密明文字段透明读取（向后兼容）
- `encrypt(text)` / `decrypt(text)` / `decrypt_list(rows, fields)` / `decrypt_dict(row, fields)` / `is_enabled()` 五个公开 API
- `database.py` 全面接入：`messages.content`、`messages.original_text`、`messages.translated_text`、`messages.quoted_text`、`address_monitors.address`、`address_monitors.remark`、`monitor_transactions.address`、`monitor_transactions.counterparty`

**生成密钥命令（服务器端）**：
```bash
dd if=/dev/urandom bs=32 count=1 2>/dev/null | base64 | tr '+/' '-_' | tr -d '='
# 输出 44 字符写入 .env：DB_ENCRYPT_KEY=xxxxxx...
```

**验证方式**：
```sql
-- 新消息应以 ENC:: 开头
SELECT msg_id, content FROM messages ORDER BY msg_id DESC LIMIT 3;
```

#### 2. 群聊引用非 Bot 消息静默忽略

**问题**：管理员在群聊中引用任意消息时，Bot 会回复「❌ 无法识别目标用户 ID」，对普通群成员可见，体验极差。

**修复**：`handlers/customer_service.py` 中 `target_user_id` 为空时，检查 `chat.type` 是否为 `group`/`supergroup`，若是则直接 `return`，不发送任何回复。

**代码位置**：`handlers/customer_service.py` 管理员回复逻辑段

#### 3. `/start` 菜单 UX 重设计

**改动**：`handlers/start.py`

- 管理员与普通用户分别显示专属菜单文案与按钮
- 按钮改为两列排布（`InlineKeyboardButton` 每行两个）
- 去除「你的 ID: xxx (非管理员)」调试按钮
- 管理员菜单：Web 管理后台、系统设置、现货行情、合约行情、监听钱包、我的列表、邮箱设置
- 普通用户菜单：联系人工客服（置顶）+ 行情/监听/邮箱功能

#### 4. 管理后台入口改为 OTP 链接

**问题**：`admin_dashboard` 回调生成的链接包含明文 `WEB_ADMIN_TOKEN`，存在泄露风险。

**修复**：`handlers/start.py` 中 `admin_dashboard` 回调改为调用 `web_admin.generate_otp()`，生成 5 分钟有效、一次性 OTP 参数链接（`?otp=xxx`）。

#### 5. 链上监听面板数据隔离与 UI 重设计

**问题**：`get_all_monitors_for_admin` 返回所有用户的监听地址，管理员后台会看到其他用户的数据。

**数据库层改动**：
- `address_monitors` 新增 `owner_type TEXT DEFAULT 'user'`
- 迁移时自动将管理员地址标记为 `admin_own`
- 新增 `get_user_query_addresses()` — 返回非管理员的查询地址
- 新增 `claim_monitor_as_admin(monitor_id)` — 将指定地址转为管理员自有
- 修改 `get_all_monitors_for_admin()` — 只返回 `owner_type = 'admin_own'` 的地址

**API 层改动**：
- 新增 `GET /api/monitors/user_queries`
- 新增 `POST /api/monitors/{id}/claim`

**前端 UI 改动**：
- 监听地址列表从表格改为**卡片式布局**，每张卡片直接显示盈亏概览
- 点击「展开近期交易记录」在卡片内内联编辑每笔（收入/支出原因、付款方、收款方）
- 新增「用户查询地址」独立分区，支持一键「⊕ 转为我的监听」
- 去除勾选列（选择功能保留用于导出统计）

---

### 2026-03-25 修复（v2.4）

#### 1. 群聊工作台点击群组无响应（`selectGroupRoom` 缺失）

**问题**：群组管理页左侧群列表每个卡片绑定了 `@click="selectGroupRoom(g)"`，但 Vue methods 中**从未定义该方法**，点击群组时 Vue 静默报错，`selectedGroupRoomId` 永远为 `null`，右侧聊天区域始终显示占位状态，消息列表空白。

**修复**：在 `methods` 中新增 `selectGroupRoom(g)` 方法，位于 `refreshGroupRoomMessages` 之前：
```js
async selectGroupRoom(g) {
    if (!g || !g.group_id) return;
    if (this.selectedGroupRoomId !== g.group_id) {
        this.groupRoomMessages = [];
        this.groupRoomInput = '';
        this.groupRoomReplyingTo = null;
    }
    this.selectedGroupRoomId = g.group_id;
    this.selectedGroupRoomTitle = g.group_title || String(g.group_id);
    this.selectedGroupRevokeSync = !!(g.delete_on_revoke);
    await this.refreshGroupRoomMessages();
},
```

**代码位置**：`admin_new.html` `selectGroupRoom()` 方法

**快速排查**：浏览器 F12 → Console，点击群组后看是否有 `selectGroupRoom is not a function` 报错

---

#### 2. 群消息 API 报错 `no such column: avatar_file_id`

**问题**：`GET /api/group_messages?group_id=xxx` 返回 `{"status": "error", "message": "no such column: avatar_file_id"}`，导致群聊工作台消息列表永远空白。

**根本原因**：`database.py` 的 `get_group_messages()` SQL JOIN 了 `group_members` 表并查询其 `avatar_file_id` 列，但 `init_db()` 迁移逻辑**只给 `group_messages` 添加了该列，漏掉了 `group_members` 表**。

**快速排查**：
```powershell
# 1. 直接调 API 看报错
Invoke-WebRequest -Uri 'http://127.0.0.1:8080/api/group_messages?group_id=xxx&limit=1' -UseBasicParsing

# 2. 检查 group_members 表结构
python -c "import sqlite3; conn=sqlite3.connect('d:/开发/tgbot/crypto_bot.sqlite3'); c=conn.cursor(); c.execute('PRAGMA table_info(group_members)'); [print(r) for r in c.fetchall()]; conn.close()"
```

**修复**：已手动执行并补入 `init_db()` 迁移逻辑：
```sql
ALTER TABLE group_members ADD COLUMN avatar_file_id TEXT DEFAULT NULL;
```

**数据库依赖链**：
```
get_group_messages()
  └─ LEFT JOIN group_members ON user_id
       └─ 需要 group_members.avatar_file_id ← 本次补加的列

save_group_member(avatar_file_id=...)
  └─ 写入 group_members.avatar_file_id

track_group_seen() [group_monitor.py]
  └─ 调用 save_group_member(avatar_file_id=_avatar)
```

**代码位置**：`database.py` `get_group_messages()` 函数、`init_db()` group_members 迁移段

---

#### 3. Vue 3 中 `this.$set` 调用报 TypeError

**问题**：`autoLoadFile()` 和 `loadFile()` 中使用 `this.$set(this.fileUrls, fileId, url)`，Vue 3 不存在此 API，运行时抛出 `TypeError: this.$set is not a function`，头像和媒体 URL 无法响应式更新，图片显示空白。

**修复**：
```js
// 修复前（Vue 2 写法）
this.$set(this.fileUrls, fileId, json.url)
this.$set(this.fileErrors, fileId, true)

// 修复后（Vue 3 展开赋值）
this.fileUrls = { ...this.fileUrls, [fileId]: json.url }
this.fileErrors = { ...this.fileErrors, [fileId]: true }
```

**代码位置**：`admin_new.html` `autoLoadFile()`、`loadFile()` 函数

---

#### 4. Vue 2 响应式头像不显示

**问题**：用户列表、群聊消息中头像加载完成后页面不重新渲染，始终显示首字缩写。

**原因**：`autoLoadFile()` 和 `loadFile()` 使用 `this.fileUrls[fileId] = url` 直接赋值，Vue 2 无法检测到新属性添加，不触发重新渲染。

**修复**：改用 `this.$set(this.fileUrls, fileId, url)` 和 `this.$set(this.fileErrors, fileId, true)`。

**代码位置**：`admin_new.html` `autoLoadFile()`、`loadFile()` 函数

#### 2. 用户列表头像 v-if 条件错误

**问题**：`v-if="user.avatar_file_id"` 只判断字段存在，URL 未加载时 `<img>` src 为 undefined，fallback 的首字缩写因条件不满足也不显示。

**修复**：改为 `v-if="user.avatar_file_id && fileUrls[user.avatar_file_id]"`，URL 加载完成前显示首字缩写。

**代码位置**：`admin_new.html` 用户列表头像区域

#### 3. 群消息时间戳显示错误

**问题**：群消息面板与群聊工作台时间戳只读 `m.timestamp`（数据库 DATETIME 字符串），但实际时间存在 `m.message_ts`（Unix 时间戳），导致时间显示为空或不正确。

**修复**：两处 `formatTime(m.timestamp)` 均改为 `formatTime(m.message_ts || m.timestamp)`，优先用 Unix 时间戳。

**代码位置**：`admin_new.html` 第 1136 行（群组管理消息历史）、第 1693 行（群聊工作台）

#### 4. `formatTime()` UTC 时区误差

**问题**：数据库存储的 DATETIME 字符串（如 `"2026-03-25 02:09:00"`）无时区标识，浏览器默认按 UTC 解析，导致时间显示比实际早 8 小时。

**修复**：`formatTime()` 对无时区标识的字符串自动补 `+08:00`（服务器运行在 UTC+8）；Unix 时间戳（10-13位数字）直接转 Date，天然无时区歧义。

**代码位置**：`admin_new.html` `formatTime()` 函数（第 3539 行）

#### 5. 群消息图片/媒体不显示

**问题**：切换到群聊工作台后，消息中的图片、视频、音频均显示为空白，`fileUrls` 中没有对应 URL。

**原因**：消息加载后没有触发 `autoLoadFile()` 预加载媒体 URL；且 `autoLoadFile` 用直接赋值不触发响应式。

**修复**：
1. `refreshGroupRoomMessages()` 末尾对所有 `m.file_id` 批量调用 `autoLoadFile()`
2. `autoLoadFile()` 改用 `$set` 触发响应式更新
3. 头像 URL 也一并批量预加载

**代码位置**：`admin_new.html` `refreshGroupRoomMessages()` 函数

**媒体类型覆盖**：photo / video / animation / voice / audio / sticker / document

#### 6. 群消息对账循环优化

**问题**：60s 间隔太慢；检查无时间限制的消息浪费 API；串行处理多群效率低；无限速保护。

**修复**：见「核心功能模块 - 信息校对对账」章节详细对比表。

**代码位置**：`main.py` `revoke_sync_loop()`、`database.py` `get_groups_with_revoke_sync()`

#### 7. 群消息头像始终为 None

**问题**：`message.from_user.photo` 在 Bot API 消息对象中永远为 None，群消息和私聊消息均无头像。

**修复**：改用 `context.bot.get_chat(user_id)` 获取完整用户信息，配合内存缓存 `_avatar_cache` 避免重复 API 调用；同步写入 `admin_inbox.avatar_file_id`。

**代码位置**：`handlers/group_monitor.py` `track_group_seen()` 函数

---

### 2026-03-24 修复

#### 5. 回复文本简洁化

**优化点**：减少表情符号、统一格式、每条消息固定包含用户名与 UserID。

**代码路径**：`handlers/customer_service.py`、`handlers/start.py`

#### 6. 媒体消息映射补齐

**问题**：管理员回复媒体转发消息时，映射表缺失导致无法识别目标用户。

**修复**：媒体转发给管理员后补充写入 `admin_message_mapping`。

**代码路径**：`handlers/customer_service.py`、`database.py`

#### 7. 群发支持无草稿直接发送

**问题**：发送按钮强依赖 `activeBroadcastDraftId`，不保存草稿无法发送。

**修复**：调整按钮禁用条件与 `submitBroadcast()` 逻辑，支持两种模式并存。

**代码路径**：`admin_new.html` Broadcast 区

---

### 2026-03-23 修复

#### 8. 贴纸渲染失败

**问题**：`.webm` 视频贴纸在 Web 端显示为空白。

**修复**：检测 URL 后缀，使用 `<video autoplay loop muted playsinline>` 渲染。

**代码位置**：`admin_new.html`

#### 9. 30 分钟后全部消息超时

**问题**：UTC 时区与本地时间混用导致所有消息被误判为已过期。

**修复**：统一使用 `datetime.utcnow()` 进行比较。

**代码位置**：`handlers/customer_service.py`

---

## 快速排查指南

### 常见症状与定位

| 症状 | 第一步排查 | 根本原因 |
|------|-----------|----------|
| 群聊工作台点击群组无反应 | F12 Console 看是否有 `is not a function` | `selectGroupRoom` 方法缺失 |
| `/api/group_messages` 返回 error | 查看 message 字段内容 | 多为 `group_members` 缺列 |
| 头像/图片不显示 | F12 Console 看是否有 `$set is not a function` | Vue 3 不支持 `this.$set` |
| 群消息列表空白但 API 正常 | 检查 `selectedGroupRoomId` 是否被赋值 | `selectGroupRoom` 未定义或未调用 |
| 时间显示差 8 小时 | 检查 `formatTime()` 是否补 `+08:00` | 数据库时间字符串无时区标识 |
| bot 启动后消息不入库 | 检查 `tracked_groups.is_monitored` | 群组监听开关未开启 |

### 数据库快速检查命令（PowerShell）

```powershell
# 检查所有表结构
python -c "import sqlite3; conn=sqlite3.connect('d:/开发/tgbot/crypto_bot.sqlite3'); c=conn.cursor(); c.execute(\"SELECT name FROM sqlite_master WHERE type='table'\"); [print(r) for r in c.fetchall()]; conn.close()"

# 检查 group_members 列
python -c "import sqlite3; conn=sqlite3.connect('d:/开发/tgbot/crypto_bot.sqlite3'); c=conn.cursor(); c.execute('PRAGMA table_info(group_members)'); [print(r) for r in c.fetchall()]; conn.close()"

# 检查 group_messages 数据量
python -c "import sqlite3; conn=sqlite3.connect('d:/开发/tgbot/crypto_bot.sqlite3'); c=conn.cursor(); c.execute('SELECT group_id, COUNT(*) FROM group_messages GROUP BY group_id'); [print(r) for r in c.fetchall()]; conn.close()"

# 检查群组监听状态
python -c "import sqlite3; conn=sqlite3.connect('d:/开发/tgbot/crypto_bot.sqlite3'); c=conn.cursor(); c.execute('SELECT group_id, group_title, is_monitored FROM tracked_groups'); [print(r) for r in c.fetchall()]; conn.close()"
```

### API 快速测试命令（PowerShell）

```powershell
# 群组列表
Invoke-WebRequest -Uri 'http://127.0.0.1:8080/api/group_listen/list?page=1&size=5' -UseBasicParsing | Select-Object -ExpandProperty Content

# 群消息（替换 group_id）
Invoke-WebRequest -Uri 'http://127.0.0.1:8080/api/group_messages?group_id=-1001991086404&limit=3' -UseBasicParsing | Select-Object -ExpandProperty Content

# 群成员
Invoke-WebRequest -Uri 'http://127.0.0.1:8080/api/group_members?group_id=-1001991086404' -UseBasicParsing | Select-Object -ExpandProperty Content
```

### 关键依赖路径

```
群聊工作台消息显示完整链路：

admin_new.html
  selectGroupRoom(g)               ← 点击群组触发（v2.4 新增）
    └─ refreshGroupRoomMessages()
         └─ GET /api/group_messages?group_id=xxx
              └─ web_admin.py → database.get_group_messages()
                   └─ SELECT group_messages JOIN group_members
                        ├─ group_messages.avatar_file_id   ← 需存在
                        └─ group_members.avatar_file_id    ← v2.4 补加，之前漏掉

群消息写入链路：

Telegram 群消息
  └─ handlers/group_monitor.py → track_group_seen()
       ├─ _get_group_config(group_id) → tracked_groups.is_monitored 必须为 1
       ├─ database.save_group_message()  → group_messages 表
       └─ database.save_group_member()  → group_members 表

头像获取链路：

track_group_seen()
  └─ context.bot.get_chat(user_id)   ← 每用户只调一次（_avatar_cache 缓存）
       └─ chat.photo.small_file_id
            ├─ save_group_message(avatar_file_id=...)  → group_messages
            ├─ save_group_member(avatar_file_id=...)   → group_members
            └─ update_user_avatar()                    → admin_inbox
```

---

## 附录

### 文件清单

| 文件 | 说明 |
|------|------|
| main.py | 系统入口、revoke_sync_loop |
| config.py | 全局配置 |
| database.py | 数据库层（20 个表，v3.3 新增 counterparty_aliases）|
| web_admin.py | Web 后台 API |
| admin_new.html | Web 前端（Vue 3） |
| handlers/customer_service.py | 客服系统 |
| handlers/group_monitor.py | 群组监听、头像缓存采集 |
| services/chain_monitor.py | 链上监听 |
| services/price_monitor.py | 行情监控 |
| services/translator.py | 翻译服务 |
| services/security_center.py | 安全核验 |
| services/field_crypto.py | 字段级 AES-256-GCM 加密（v3.2 新增）|
| handlers/start.py | /start 菜单（v3.2 重设计）|

### 外部接口

| 服务 | 端点 | 用途 |
|------|------|------|
| Telegram | api.telegram.org | Bot 消息收发 |
| TronGrid | api.trongrid.io | TRC20 交易查询 |
| BscScan | api.bscscan.com | BEP20 交易查询 |
| Binance | api.binance.com | 行情数据 |
| Gemini/DeepSeek | generativelanguage.googleapis.com / api.deepseek.com | 翻译与 LLM |

---

**文档维护**：每次功能更新或 bug 修复后，需更新本文档的版本号、修订日期和变更摘要表。
