# SPEC-2.2c: Agent 会话层移植规格

> 版本: 1.0  
> 日期: 2026-07-26  
> 对应提交: 519921b, c816441, 4c3f1dc, afe8a31（PORTING-13-COMMITS.md 项目 2/10b/10c/12 及项目 3 的否定守卫）  
> 状态: 阅读-整体设计→待 Rust 实现→待测试

---

## 一、概述

Batch 2.2c 覆盖 Agent 会话层的五项关键行为：

1. **confirm_and_execute 回复修复**（519921b）：确认执行后仅报本次执行结果，不重新拼接旧"请确认"提示
2. **会话取消词表**（c816441 (b)）：识别取消/停止词并清空 pending 状态，不执行工具
3. **短跟进指令解析**（c816441 (c)）：将"删除"、"第N个"等简短命令根据上一轮已验证回复展开为完整意图
4. **会话上下文追踪**（afe8a31）：metadata 记录 active_group_id / last_ad_query_group_id，为裸跟进指令提供群作用域
5. **否定/疑问守卫**（项目 3 的否定守卫）："不要删除"、"为什么删除"等不触发删除流

所有行为均涉及**会话状态管理**和**对话历史解析**，实现于 HTTP 层（routes/ai.rs）和 Agent 核心（crates/agent/src/core.rs）。

---

## 二、详细行为规格

### 2.1 行为 1: confirm_and_execute 回复修复

#### 2.1.1 Python 原理

**源文件**: `services/ai_agent/core.py::AIAgent.confirm_and_execute()` L452-512（commit 519921b）

**关键逻辑**（Python L489-512）：

```python
# 确认后的回复只描述本次执行结果，避免再次拼接旧的"请确认"提示。
result_messages = [
    str(result.get("message") or "").strip()
    for result in executed_results.values()
    if isinstance(result, dict) and str(result.get("message") or "").strip()
]
if has_failure and action_taken:
    result_text = "操作已部分执行，但有失败项：\n" + "\n".join(f"- {msg}" for msg in failure_messages)
elif has_failure:
    result_text = "操作未执行：\n" + "\n".join(f"- {msg}" for msg in failure_messages)
else:
    result_text = "操作已执行完毕。"
    if result_messages:
        result_text += "\n" + "\n".join(f"- {msg}" for msg in result_messages)

return AgentResponse(
    text=result_text,  # ← 关键：仅返回执行结果，不拼接 pending_response.text
    tool_calls_made=pending_response.tool_calls_made,
    tool_results=executed_results,
    is_action_taken=action_taken,
    requires_confirmation=False,
    action_summary=pending_response.action_summary,
    confidence=1.0 if not has_failure else 0.7,
)
```

**修复前的错误行为**（Rust core.rs L478，需要改正）：
```rust
text: format!("{}\n{}", result_text, pending_response.text),  // ❌ 拼接了旧提示
```

**修复要点**：
- 执行后**仅返回本次执行摘要**，不包含原 pending_response 的文本
- 三种返回文本：
  1. **部分执行** (`has_failure && action_taken`): "操作已部分执行，但有失败项：\n- 错误1\n- 错误2"
  2. **未执行** (`has_failure && !action_taken`): "操作未执行：\n- 错误1\n- 错误2"
  3. **全部成功** (`!has_failure`): "操作已执行完毕。" + 各工具消息（若有）
- 置信度: 失败时 0.7，成功时 1.0

#### 2.1.2 Rust 实现目标

**涉及文件**: `crates/agent/src/core.rs::AIAgent::confirm_and_execute()` L408-487

**当前问题**: L478 仍在拼接 `pending_response.text`，需修改为仅返回结果摘要

**修复步骤**:
1. 保留执行逻辑（L422-460）
2. 改进文本组织（L462-473）：
   - 收集各工具的 `message` 字段
   - 按 `has_failure` 和 `action_taken` 组合判定文本模式
   - 仅返回当次执行结果，不涉及 pending_response.text
3. 更新 AgentResponse 构造（L477-486）

#### 2.1.3 数据模型

- **输入**: `pending_response: AgentResponse` 含有 `tool_results` 字段（各工具的前次查询/准备结果）
- **过程**: 遍历 `tool_results` 中 `needs_confirmation=true` 的工具，重新执行
- **输出**: 新的 `AgentResponse`，text 为执行结果摘要，`requires_confirmation=false`

#### 2.1.4 边界情况与测试

| 测试名 | 场景 | 预期 |
|-------|------|------|
| test_confirm_delete_success | 删除 2 条广告，全部成功 | text="操作已执行完毕。" |
| test_confirm_delete_partial | 删除 2 条，其中 1 条失败（如已删） | text="操作已部分执行，但有失败项：\n- delete_recent_ad_messages: message_id=102 已删除" |
| test_confirm_delete_all_fail | 删除被拒绝（如 AD_DELETE_ENABLED=false） | text="操作未执行：\n- delete_recent_ad_messages: 必须先由后端查询..." |
| test_confirm_must_not_append_pending | 确认执行后，不应包含旧的"请确认"文本 | assert "请回复「确认」" not in result.text |

---

### 2.2 行为 2: 会话取消词表清空 pending

#### 2.2.1 Python 原理

**源文件**: `web_admin.py::_is_agent_cancellation_text()` L2193-2204 + `_run_ai_agent_chat()` L2419-2431

**取消词表**（精确列表，VerbatIM）:

```python
def _is_agent_cancellation_text(text: str) -> bool:
    normalized = re.sub(r"\s+", "", str(text or "").strip().lower())
    return normalized in {
        "取消",
        "取消操作",
        "不要了",
        "停止",
        "别执行",
        "不删除",
        "cancel",
        "no",
    }
```

**会话取消流程**（Python L2419-2431）:

```python
pending = session.get('pending')
if pending is not None and _is_agent_cancellation_text(input_text):
    from services.ai_agent.schema import AgentResponse
    session['pending'] = None  # ← 清空 pending
    return AgentResponse(
        text='已取消待确认操作，没有执行任何动作。',
        tool_calls_made=pending.tool_calls_made,
        tool_results={},  # ← 无执行结果
        is_action_taken=False,
        requires_confirmation=False,
        action_summary=pending.action_summary,
        confidence=1.0,
    )
```

**流程**:
1. 检查是否存在 `pending` 操作（待确认）
2. 检查用户输入是否匹配取消词表（归一化：去空格、转小写、模式匹配）
3. 若匹配，清空 pending，返回"已取消"响应（仅返回原 tool_calls_made 和 action_summary，无执行）
4. 若不匹配或无 pending，继续正常处理

#### 2.2.2 Rust 实现目标

**涉及文件**: `crates/http/src/routes/ai.rs::ai_chat()` + Agent session 中间件

**实现位置**:
- 会话层（HTTP 层）拦截，不应下沉到 Agent 核心
- 需要会话存储机制（类似 Python 的 `_ai_agent_sessions` 字典）

**实现逻辑**:

```rust
// 伪代码
pub fn is_agent_cancellation_text(text: &str) -> bool {
    let normalized = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    
    matches!(
        normalized.as_str(),
        "取消" | "取消操作" | "不要了" | "停止" | 
        "别执行" | "不删除" | "cancel" | "no"
    )
}

// 在 ai_chat 处理逻辑中
if let Some(pending) = session.get_pending() {
    if is_agent_cancellation_text(&input_text) {
        session.clear_pending();
        return AgentResponse {
            text: "已取消待确认操作，没有执行任何动作。".into(),
            tool_calls_made: pending.tool_calls_made.clone(),
            tool_results: HashMap::new(),
            is_action_taken: false,
            requires_confirmation: false,
            action_summary: pending.action_summary.clone(),
            confidence: 1.0,
            ..Default::default()
        };
    }
}
```

#### 2.2.3 数据模型

- **会话结构** (需在 HTTP 层定义):
  ```rust
  pub struct AgentSession {
      pub ctx: AgentContext,
      pub pending: Option<AgentResponse>,  // 待确认操作
      pub updated_at: i64,  // 时间戳
  }
  ```
- **取消词表**: 8 个固定字符串，不可扩展（安全关键）

#### 2.2.4 边界情况与测试

| 测试名 | 场景 | 预期 |
|-------|------|------|
| test_cancellation_with_pending | 删除待确认，用户回复"取消" | text="已取消...", requires_confirmation=false, is_action_taken=false |
| test_cancellation_case_insensitive | 待确认，用户回复"CANCEL" 或"取消" | 应识别（归一化后匹配） |
| test_cancellation_ignores_spaces | 待确认，用户回复"取 消" 或" 停 止 " | 应识别（去空格后匹配） |
| test_cancellation_without_pending | 未有待确认，用户回复"取消" | 应继续正常流程（不判定为取消） |
| test_cancellation_partial_match_rejected | 待确认，用户回复"不要" 或"取消一下" | 不应触发取消（取消词表为完全匹配） |

---

### 2.3 行为 3: 短跟进指令解析

#### 2.3.1 Python 原理

**源文件**: `services/ai_agent/core.py::AIAgent._resolve_contextual_command()` L858-906

**核心逻辑** — 两类短命令的展开（commit c816441）:

**情况 A: 删除指令简写**（L880-885）

```python
if normalized in {"删除", "删掉", "执行删除", "清理", "处理掉"} and any(
    marker in previous for marker in ("条合格广告", "具备删除权限", "广告记录")
):
    last_ad_group_id = metadata.get("last_ad_query_group_id")
    suffix = f" 群 {int(last_ad_group_id)}" if str(last_ad_group_id or "").strip() else ""
    return f"删除最近24小时的广告{suffix}"
```

**触发条件**:
- 用户输入归一化后为 {"删除", "删掉", "执行删除", "清理", "处理掉"} 之一
- 上一轮 Assistant 消息中包含"条合格广告"或"具备删除权限"或"广告记录"任一标志
- 必须有 `last_ad_query_group_id` 上下文（避免跨群误删）

**返回**:
- 若有群ID：`"删除最近24小时的广告 群 -1002222222222"`
- 若无群ID：`"删除最近24小时的广告"`

**情况 B: 群序号跟进指令**（L887-904）

```python
ordinal_match = re.search(r"第\s*(\d{1,2}|一|二|三|四|五|六|七|八|九|十)\s*个", text)
terse_ordinal = normalized in {"第一个", "第二个", "第三个", "第四个", "第五个"}
wants_group_messages = bool(ordinal_match) and (
    terse_ordinal or any(word in normalized for word in ("群", "消息", "查看", "打开", "看看"))
)
if wants_group_messages and "群组：" in previous:
    token = ordinal_match.group(1)
    chinese_numbers = {
        "一": 1, "二": 2, "三": 3, "四": 4, "五": 5,
        "六": 6, "七": 7, "八": 8, "九": 9, "十": 10,
    }
    index = chinese_numbers.get(token, int(token) if token.isdigit() else 0)
    group_section = previous.split("群组：", 1)[-1].split("频道：", 1)[0]
    group_ids = [int(value) for value in re.findall(r"\((-?\d{5,})\)", group_section)]
    if 1 <= index <= len(group_ids):
        selected_group_id = group_ids[index - 1]
        metadata["active_group_id"] = selected_group_id
        return f"查看群 {selected_group_id} 最近群消息"
```

**触发条件**:
- 用户输入中含"第N个"（N 为 1-99 数字或汉字一-十）
- **且** (仅"第一个"等极简形式 **或** 含"群"、"消息"、"查看"、"打开"、"看看")
- 上一轮 Assistant 消息中包含"群组："标志（表示已列出群组列表）

**群组解析**:
1. 从 previous 中截取"群组："和"频道："之间的部分
2. 正则提取所有`(-?\d{5,})`型群ID（Telegram 负数ID）
3. 按序号 index 映射到实际群ID
4. 同时更新 `metadata["active_group_id"]` 为选中群ID

**返回**: `"查看群 -1002222222222 最近群消息"`

#### 2.3.2 Rust 实现目标

**涉及文件**: `crates/agent/src/core.rs` 或新建 `crates/agent/src/contextual_command.rs`

**实现函数签名**:

```rust
/// 根据对话历史上下文展开简短跟进指令
pub fn resolve_contextual_command(
    user_text: &str,
    metadata: &mut HashMap<String, serde_json::Value>,
    previous_assistant_text: &str,
) -> String {
    // 情况 A: 删除简写
    // 情况 B: 群序号跟进
    // 默认: 返回原文本
}
```

**关键点**:
- 接收原始用户文本和上一轮 Assistant 消息
- 返回展开后的意图字符串或原文本
- 修改 `metadata` 中的 `active_group_id`（side-effect，需与 context 绑定）
- 正则表达式须与 Python 版本 bit-level 一致

#### 2.3.3 数据模型

- **输入**:
  - `user_text: String` — 原始用户输入
  - `ctx.metadata: HashMap` — 会话上下文（含 last_ad_query_group_id）
  - `previous: String` — 上一轮 Assistant 输出
- **处理**:
  - 字符串归一化: 去空格、转小写
  - 正则匹配: 第N个模式、群组列表、群ID 提取
  - 汉字数字映射: 一-十 → 1-10
- **输出**:
  - 展开后的完整指令或原文本
  - 修改 metadata 中的 `active_group_id`

#### 2.3.4 正则表达式（必须精确匹配）

| 用途 | Python 正则 | 说明 |
|------|----------|------|
| 群序号提取 | `r"第\s*(\d{1,2}\|一\|二\|三\|四\|五\|六\|七\|八\|九\|十)\s*个"` | 第 + 可选空格 + 数字或汉字 + 可选空格 + 个 |
| 群组标志 | `"群组："` | 上一轮回复中的分节标记 |
| 频道标志 | `"频道："` | 用于截断群组部分 |
| 群ID 提取 | `r"\((-?\d{5,})\)"` | 括号内的负数 ID（Telegram 群ID 格式） |

#### 2.3.5 边界情况与测试

| 测试名 | 场景 | 预期 |
|-------|------|------|
| test_terse_delete_with_ad_context | 历史: "最近找到2条广告", 输入: "删除" | 展开为"删除最近24小时的广告" + 群ID（若有） |
| test_terse_delete_without_context | 无历史或历史无广告标志, 输入: "删除" | 不展开，返回原文"删除" |
| test_ordinal_group_selection | 历史: "群组：\n1. A (-1001)\n2. B (-1002)", 输入: "第二个" | 展开为"查看群 -1002222222222 最近群消息", 更新 active_group_id=-1002 |
| test_ordinal_out_of_range | 历史: 列2个群, 输入: "第五个" | 不展开，返回原文"第五个" |
| test_ordinal_chinese_numbers | 输入: "第三个", "第十个", 输入汉字 | 应正确映射 |
| test_contextual_command_case_insensitive | 输入: "删除", "DELE", 混合大小写 | 应归一化后匹配 |

---

### 2.4 行为 4: 会话元数据持久化与继承

#### 2.4.1 Python 原理

**源文件**: `web_admin.py::_run_ai_agent_chat()` L2320-2440，`services/ai_agent/core.py::_resolve_contextual_command()` L864-906

**元数据字段**（在 `ctx.metadata` 中维护）:

```python
ctx.metadata = {
    'tenant_id': tenant_id,
    'merchant_id': scope.get('merchant_id'),
    'role': scope.get('role'),
    'chat_id': chat_id,
    'group_id': chat_id,
    'active_group_id': None,  # ← 用户最后选中的群（如"第二个"）
    'last_ad_query_group_id': None,  # ← 最后一次 query_recent_ad_messages 的群ID
}
```

**active_group_id 的设置时机** (commit afe8a31):

1. **用户显式指定群ID** (`_resolve_contextual_command` L866-868):
   ```python
   explicit_group = re.search(r"(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
   if explicit_group:
       metadata["active_group_id"] = int(explicit_group.group(1))
   ```

2. **用户选择序号群** (`_resolve_contextual_command` L902-903):
   ```python
   if 1 <= index <= len(group_ids):
       selected_group_id = group_ids[index - 1]
       metadata["active_group_id"] = selected_group_id  # ← 记录选中群
   ```

**last_ad_query_group_id 的设置时机** (commit afe8a31):

1. **广告查询工具执行后** (core.py L141-143):
   ```python
   if tool_name == "query_recent_ad_messages":
       ctx.metadata["last_ad_query_group_id"] = exec_params.get("group_id")
   ```

2. **准备删除前的查询** (core.py L571-572):
   ```python
   try:
       query_result = await query_tool.execute(query_params, context=ctx)
       ctx.metadata["last_ad_query_group_id"] = query_params.get("group_id")
   ```

**继承语义** — "裸"跟进指令（如仅"删除"）的群作用域继承（core.py L883-885）:

```python
# 用户输入"删除"时，优先使用 last_ad_query_group_id（上次查询的群）
last_ad_group_id = metadata.get("last_ad_query_group_id")
suffix = f" 群 {int(last_ad_group_id)}" if str(last_ad_group_id or "").strip() else ""
return f"删除最近24小时的广告{suffix}"
```

**会话管理** (web_admin.py L2340-2398):

```python
session_key = _agent_session_key(tenant_id=tenant_id, user_id=user_id, scope=scope)
session = _ai_agent_sessions.get(session_key)
if not session:
    ctx = AgentContext(...)  # 新会话，初始化 metadata
    session = {'ctx': ctx, 'pending': None, 'updated_at': time.time()}
    _ai_agent_sessions[session_key] = session
else:
    ctx = session['ctx']
    # 会话复用时更新租户/角色信息，但保留 active_group_id 和 last_ad_query_group_id
```

**会话失效** (web_admin.py L2392-2397):
- 超过 6 小时无操作则清除
- 同一用户/租户的 session key 保证唯一

#### 2.4.2 Rust 实现目标

**涉及文件**: 
- `crates/http/src/routes/ai.rs::ai_chat()` - 会话管理
- `crates/agent/src/core.rs::AIAgent` - metadata 传递

**会话存储**:

```rust
// 在 AppState 中存储
pub struct AgentSession {
    pub ctx: AgentContext,
    pub pending: Option<AgentResponse>,
    pub updated_at: i64,
}

// 会话字典（使用 DashMap 或 Arc<Mutex<HashMap>>）
pub sessions: DashMap<String, AgentSession>,
```

**会话 key 生成**:

```rust
fn session_key(tenant_id: &str, user_id: i64, merchant_id: Option<&str>) -> String {
    format!("{}:{}:{}", tenant_id, user_id, merchant_id.unwrap_or(""))
}
```

**metadata 字段初始化和更新**:

```rust
// 新会话初始化（来自 HTTP 请求）
let mut metadata = HashMap::new();
metadata.insert("tenant_id".into(), json!(tenant_id));
metadata.insert("merchant_id".into(), json!(merchant_id));
metadata.insert("role".into(), json!(role));
metadata.insert("chat_id".into(), json!(chat_id));
metadata.insert("group_id".into(), json!(chat_id));
// active_group_id 和 last_ad_query_group_id 保持 null 或不存在

// 现有会话复用时，保留 active_group_id 和 last_ad_query_group_id
ctx.metadata.insert("tenant_id".into(), json!(tenant_id));  // 更新
ctx.metadata.insert("merchant_id".into(), json!(merchant_id));  // 更新
// ctx.metadata 中的 "active_group_id" 和 "last_ad_query_group_id" 保持不变
```

**工具执行后的 metadata 更新**:

```rust
// 在 process_message 的工具循环中（类似 Python L141-143）
if tool_name == "query_recent_ad_messages" {
    if let Some(group_id) = exec_params.get("group_id").and_then(|v| v.as_i64()) {
        ctx.metadata.insert("last_ad_query_group_id".into(), json!(group_id));
    }
}
```

**会话过期清理**:

```rust
// 启动后台任务，定期清理
tokio::spawn(async move {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;  // 5 分钟检查一次
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - 6 * 3600;
        sessions.retain(|_, session| session.updated_at > cutoff);
    }
});
```

#### 2.4.3 数据模型

- **AgentContext 中的 metadata**:
  ```rust
  metadata: HashMap<String, Value> {
      "tenant_id": String,
      "merchant_id": Option<i64 or String>,  // UUID in new Rust, legacy int in bridge
      "role": String,  // "platform_admin", "merchant_owner", etc.
      "chat_id": Option<i64>,  // Telegram chat_id (current context)
      "group_id": Option<i64>,  // Alias for chat_id
      "active_group_id": Option<i64>,  // User-selected group from ordinal pick
      "last_ad_query_group_id": Option<i64>,  // Last query_recent_ad_messages scope
  }
  ```

#### 2.4.4 边界情况与测试

| 测试名 | 场景 | 预期 |
|-------|------|------|
| test_session_persistence | 用户A 查询群1，再查询群2，再"删除"（无群ID） | 第一次"删除"使用群2，第二次"删除"使用群1（跨消息保留） |
| test_session_isolation | 用户A 和用户B 同时操作同一租户 | session key 不同，互不影响 |
| test_session_expiry | 用户会话6小时无操作 | 自动清理，下次操作新建会话 |
| test_active_group_id_inheritance | 用户选"第二个"（群ID -1002），后续"这个群查看消息" | 应使用 -1002 |
| test_last_ad_query_group_id_override | 先查群1广告，后查群2广告，再"删除" | 应使用群2的scope |
| test_metadata_update_on_new_session | 新会话，前端提供 chat_id | metadata 中 group_id 应为该值 |

---

### 2.5 行为 5: 否定/疑问守卫

#### 2.5.1 Python 原理

**源文件**: `services/ai_agent/core.py::AIAgent._is_recent_ad_delete_request()` L514-522（项目 3 否定守卫部分）

**守卫逻辑**（归一化后的文本模式匹配）:

```python
@staticmethod
def _is_recent_ad_delete_request(text: str) -> bool:
    normalized = re.sub(r"\s+", "", str(text or "").lower())
    has_delete = any(word in normalized for word in ("删除", "清理", "移除", "删掉", "执行删除"))
    has_ad = any(word in normalized for word in ("广告", "垃圾信息", "牛皮癣"))
    is_negated = any(word in normalized for word in ("不要删除", "别删除", "禁止删除", "无需删除", "不需要删除"))
    is_question = normalized.startswith(("为什么", "为何", "怎么会"))
    requests_single = "一条" in normalized and not any(word in normalized for word in ("最近", "24小时", "小时内", "今天"))
    return has_delete and has_ad and not is_negated and not is_question and not requests_single
```

**四个守卫条件** (均使用 NOT 逻辑，任一触发则拒绝):

1. **否定守卫** (`is_negated`): 输入包含以下任一短语则拒绝
   - "不要删除"、"别删除"、"禁止删除"、"无需删除"、"不需要删除"
   - 示例："不要删除广告"、"禁止删除这些消息" → **不**触发删除

2. **疑问守卫** (`is_question`): 输入以以下短语开头则拒绝
   - "为什么"、"为何"、"怎么会"
   - 示例："为什么要删除广告？"、"为何删除成功了？" → **不**触发删除

3. **单条守卫** (`requests_single`): 输入含"一条"但不含时间限定词则拒绝
   - 无效示例："删除一条广告" → **不**触发（应使用批量删除）
   - 有效示例："删除最近一条广告"、"删除今天一条垃圾" → 触发

4. **基础守卫** (`has_delete && has_ad`): 必须同时包含删除动作词和广告标志
   - "删除"、"清理"、"移除"、"删掉"、"执行删除"
   - "广告"、"垃圾信息"、"牛皮癣"

**执行流程** (core.py L524-530):

```python
async def _prepare_recent_ad_deletion(
    self,
    user_text: str,
    ctx: AgentContext,
) -> Optional[AgentResponse]:
    if not self._is_recent_ad_delete_request(user_text):  # ← 所有守卫在此检查
        return None  # 不触发删除流，继续正常 LLM 处理
    
    # ... 开始删除准备流程
```

#### 2.5.2 Rust 实现目标

**涉及文件**: `crates/agent/src/core.rs` 或新建 `crates/agent/src/deletion_guard.rs`

**实现函数**:

```rust
/// 检查用户输入是否真正要求删除最近广告
/// 
/// 返回 false 表示删除请求被守卫拒绝或不符合格式
pub fn is_recent_ad_delete_request(text: &str) -> bool {
    let normalized = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    
    // 1. 基础检查：必须同时含删除词和广告词
    let has_delete = ["删除", "清理", "移除", "删掉", "执行删除"]
        .iter()
        .any(|w| normalized.contains(w));
    let has_ad = ["广告", "垃圾信息", "牛皮癣"]
        .iter()
        .any(|w| normalized.contains(w));
    
    if !has_delete || !has_ad {
        return false;
    }
    
    // 2. 否定守卫：含否定词则拒绝
    let is_negated = ["不要删除", "别删除", "禁止删除", "无需删除", "不需要删除"]
        .iter()
        .any(|w| normalized.contains(w));
    if is_negated {
        return false;
    }
    
    // 3. 疑问守卫：以疑问词开头则拒绝
    let is_question = ["为什么", "为何", "怎么会"]
        .iter()
        .any(|w| normalized.starts_with(w));
    if is_question {
        return false;
    }
    
    // 4. 单条守卫：仅"一条"无时间限定则拒绝
    let requests_single = normalized.contains("一条") && 
        !["最近", "24小时", "小时内", "今天"]
            .iter()
            .any(|w| normalized.contains(w));
    if requests_single {
        return false;
    }
    
    true
}
```

#### 2.5.3 数据模型

- **输入**: `text: &str` — 用户原始输入
- **处理**:
  - 归一化: 小写 + 去空格
  - 字符串模式匹配（非 regex，简单 contains/starts_with）
  - 短语完全匹配（词表固定，不模糊）
- **输出**: `bool` — 是否应进入删除流程

#### 2.5.4 字词表（须精确匹配，不可更改）

| 类别 | 字词表 | 说明 |
|------|--------|------|
| 删除动作 | 删除、清理、移除、删掉、执行删除 | 任一匹配 |
| 广告标志 | 广告、垃圾信息、牛皮癣 | 任一匹配 |
| 否定短语 | 不要删除、别删除、禁止删除、无需删除、不需要删除 | 任一匹配则拒绝 |
| 疑问短语 | 为什么、为何、怎么会 | 开头匹配则拒绝 |
| 时间词 | 最近、24小时、小时内、今天 | 用于解除"一条"限制 |

#### 2.5.5 边界情况与测试

| 测试名 | 输入 | 预期 | 原因 |
|-------|------|------|------|
| test_negation_guard_1 | "不要删除广告" | 拒绝 | 包含"不要删除" |
| test_negation_guard_2 | "禁止删除这条垃圾信息" | 拒绝 | 包含"禁止删除" |
| test_question_guard_1 | "为什么要删除这些广告？" | 拒绝 | 以"为什么"开头 |
| test_question_guard_2 | "为何删除成功了？" | 拒绝 | 以"为何"开头 |
| test_question_guard_3 | "怎么会没有广告" | 拒绝 | 以"怎么会"开头 + 无删除动作 |
| test_single_item_guard | "删除一条广告" | 拒绝 | 含"一条"无时间限定 |
| test_single_item_allowed | "删除最近一条广告" | 允许 | 含"一条"+"最近" |
| test_basic_allowed | "删除最近24小时广告" | 允许 | 含删除+广告，无否定/疑问/单条 |
| test_case_insensitive | "删除 最近 广告" | 允许 | 归一化后应匹配 |
| test_no_ad_keyword | "删除消息" | 拒绝 | 无"广告"等关键字 |
| test_no_delete_keyword | "这些广告糟糕" | 拒绝 | 无删除动作词 |

---

## 三、Rust 实现集成路径

### 3.1 代码组织

**推荐结构**:

```
crates/agent/src/
├── core.rs (现有，需修改 confirm_and_execute + process_message 调用 resolve_contextual_command)
├── deletion_guard.rs (新文件，is_recent_ad_delete_request 函数)
├── contextual_command.rs (新文件，resolve_contextual_command 函数)
├── session.rs (新文件，会话管理结构和工具)
└── ...

crates/http/src/
├── routes/ai.rs (修改 ai_chat 处理器，集成会话和决策逻辑)
└── ...
```

### 3.2 集成要点

#### 3.2.1 process_message 调用链

```
ai_chat (HTTP handler)
  ├─> 从会话存储获取或创建 AgentSession
  ├─> 检查 is_agent_cancellation_text (→ 返回取消响应或继续)
  ├─> 调用 resolve_contextual_command 展开短指令
  ├─> 调用 agent.process_message(routed_text, ctx)
  │   └─> 内部调用 _is_recent_ad_delete_request (守卫)
  │       ├─> 调用 _prepare_recent_ad_deletion 或继续 LLM 处理
  │       └─> 更新 ctx.metadata 中的 last_ad_query_group_id
  ├─> 若需确认，保存到 session.pending
  └─> 返回 AgentResponse
```

#### 3.2.2 Rust 伪代码架构

```rust
pub async fn ai_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AiChatRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");
    let input_text = body.content.as_deref().unwrap_or("").trim();
    
    if input_text.is_empty() {
        return Err(ApiError::BadRequest("input required".into()));
    }
    
    // 1. 获取或创建会话
    let session_key = format!("{}:{}:{}", 
        tenant_id, 
        body.user_id.unwrap_or(0),
        scope.merchant_id.as_deref().unwrap_or("")
    );
    
    let mut session = state
        .agent_sessions
        .entry(session_key.clone())
        .or_insert_with(|| create_new_session(...));
    
    session.updated_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    
    // 2. 检查取消
    if let Some(ref pending) = session.pending {
        if is_agent_cancellation_text(input_text) {
            session.pending = None;
            return Ok(Json(agent_response_to_json(&AgentResponse {
                text: "已取消待确认操作，没有执行任何动作。".into(),
                tool_calls_made: pending.tool_calls_made.clone(),
                tool_results: HashMap::new(),
                is_action_taken: false,
                requires_confirmation: false,
                action_summary: pending.action_summary.clone(),
                confidence: 1.0,
                ..Default::default()
            })));
        }
    }
    
    // 3. 检查确认
    if let Some(pending) = session.pending.take() {
        if is_agent_confirmation_text(input_text) {
            let agent = state.agent.clone();
            let result = agent.confirm_and_execute(&pending, &session.ctx).await;
            return Ok(Json(agent_response_to_json(&result)));
        }
    }
    
    // 4. 展开短指令
    let last_assistant_text = session.ctx
        .conversation_history
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    
    let routed_text = resolve_contextual_command(
        input_text,
        &mut session.ctx.metadata,
        &last_assistant_text
    );
    
    // 5. 调用 Agent
    let agent = state.agent.clone();
    let response = agent.process_message(&routed_text, &mut session.ctx, "gemini-2.5-flash-lite").await;
    
    // 6. 保存待确认
    if response.requires_confirmation {
        session.pending = Some(response.clone());
    } else {
        session.pending = None;
    }
    
    // 7. 返回
    Ok(Json(agent_response_to_json(&response)))
}
```

### 3.3 与现有 Rust Agent 的适配

- **process_message**: 需在开头调用 `resolve_contextual_command` 来展开输入
- **_is_recent_ad_delete_request**: 实现为 standalone 函数（非 method），在 `_prepare_recent_ad_deletion` 前调用
- **confirm_and_execute**: 改进文本拼接逻辑（L478），仅返回执行摘要
- **metadata 管理**: 确保 AgentContext 中的 metadata 在工具执行后被正确更新

---

## 四、实现检查清单

### 4.1 代码文件清单

- [ ] `crates/agent/src/deletion_guard.rs` — 新建，is_recent_ad_delete_request
- [ ] `crates/agent/src/contextual_command.rs` — 新建，resolve_contextual_command
- [ ] `crates/agent/src/session.rs` — 新建或扩展，AgentSession 结构体
- [ ] `crates/agent/src/core.rs` — 修改 confirm_and_execute 文本组织 (L462-473)
- [ ] `crates/http/src/routes/ai.rs` — 重写 ai_chat，集成会话管理

### 4.2 测试覆盖清单

#### confirm_and_execute 回复修复 (5 个测试)

- [ ] test_confirm_delete_success — 删除全部成功
- [ ] test_confirm_delete_partial — 删除部分失败
- [ ] test_confirm_delete_all_fail — 删除全部失败
- [ ] test_confirm_must_not_append_pending — 不拼接旧文本
- [ ] test_confirm_other_tools — 非删除工具的确认

#### 取消词表 (5 个测试)

- [ ] test_cancellation_with_pending — 待确认时取消
- [ ] test_cancellation_case_insensitive — 大小写无关
- [ ] test_cancellation_ignores_spaces — 空格无关
- [ ] test_cancellation_without_pending — 无待确认时忽略
- [ ] test_cancellation_word_exact_match — 完全匹配

#### 短跟进指令 (6 个测试)

- [ ] test_terse_delete_with_context — "删除"展开
- [ ] test_terse_delete_without_context — "删除"不展开
- [ ] test_ordinal_group_selection — "第N个"映射
- [ ] test_ordinal_out_of_range — 序号越界
- [ ] test_ordinal_chinese_numbers — 汉字数字映射
- [ ] test_ordinal_missing_marker — 无群组列表时不展开

#### 会话上下文 (6 个测试)

- [ ] test_session_persistence — 会话复用保留上下文
- [ ] test_session_isolation — 用户间隔离
- [ ] test_session_expiry — 6 小时过期
- [ ] test_active_group_id_inheritance — active_group_id 继承
- [ ] test_last_ad_query_group_id_override — 覆盖规则
- [ ] test_metadata_update_on_execute — 工具执行后更新

#### 否定/疑问守卫 (10 个测试)

- [ ] test_negation_guard_1 ~ 2 — 否定词拒绝
- [ ] test_question_guard_1 ~ 3 — 疑问词拒绝
- [ ] test_single_item_guard — 单条拒绝
- [ ] test_basic_allowed — 正常请求允许
- [ ] test_case_insensitive — 归一化
- [ ] test_no_keywords — 缺关键词拒绝

### 4.3 跨模块集成检查

- [ ] 会话存储与 AppState 集成
- [ ] metadata 与 AgentContext 同生命周期
- [ ] 正则表达式与 Python 版本 bit-level 一致
- [ ] 字词表与 Python 版本完全相同
- [ ] 时间戳格式与 Python 兼容
- [ ] 会话 key 生成逻辑一致

### 4.4 性能与可靠性检查

- [ ] 会话过期清理不阻塞主线程
- [ ] 会话存储大小有上限（如最多 300 个活跃会话）
- [ ] metadata 更新无竞态条件
- [ ] 重复取消或确认不导致副作用

---

## 五、已知开放问题与协调项

1. **会话存储后端**
   - Python 使用内存字典 `_ai_agent_sessions`
   - Rust 应使用 DashMap 或 Arc<Mutex<HashMap>>？还是持久化？
   - 建议：暂用 DashMap（线程安全的内存字典），后续可升级为 Redis

2. **metadata 中群ID 的类型**
   - Python 统一为 int
   - Rust 中 Telegram ID 应为 i64
   - 需确认 JSON 序列化时的格式（字符串还是数字）

3. **会话 key 中 merchant_id 的表示**
   - Python 使用原始 merchant_id（整数）
   - Rust 迁移后 merchant_id 可能为 UUID
   - 需确认 key 生成时的转换策略

4. **短指令展开的副作用**
   - resolve_contextual_command 修改 metadata（active_group_id）
   - 需确认修改是否应立即持久化？还是仅在内存中？

5. **正则表达式库选择**
   - Python: re 标准库
   - Rust: regex crate（已在 Cargo.toml 中）
   - 需验证 regex crate 的行为与 Python re 一致

---

## 六、参考资源

- **Python 源代码**
  - services/ai_agent/core.py (commits 519921b, c816441, afe8a31)
  - web_admin.py (commits c816441, _run_ai_agent_chat 函数)
  
- **Rust 源代码**
  - crates/agent/src/core.rs (confirm_and_execute, process_message)
  - crates/http/src/routes/ai.rs (ai_chat 处理器)
  - crates/agent/src/schema.rs (AgentContext, AgentResponse)

- **测试参考**
  - tests/test_agent_recent_ad_delete.py (test_terse_delete_inherits_previous_ad_query 等)
  - tests/test_ai_agent_llm_and_ad_delete.py (test_numbered_group_followup_resolves_to_verified_chat_id, test_agent_cancellation_phrases_are_deterministic 等)

---

## 七、审查检查表（Rust 实现者用）

实现完成后，请验证以下内容：

### 代码质量
- [ ] 所有函数有文档注释（中文或英文）
- [ ] 错误类型使用 Result 或 anyhow::Error
- [ ] 无 unwrap()（除非有明确的 panic safety 理由）
- [ ] 格式化: `cargo fmt`
- [ ] 检查: `cargo clippy`

### 功能正确性
- [ ] 所有 32 个测试通过
- [ ] 字词表与 Python 版本完全一致（可用 diff 校验）
- [ ] 正则表达式行为与 Python 版本一致
- [ ] 会话隔离与复用符合预期
- [ ] metadata 更新序列正确

### 安全性
- [ ] 否定/疑问守卫生效（无绕过路径）
- [ ] 会话 key 生成无重复或冲突
- [ ] 过期会话正确清理
- [ ] 无 SQL 注入或序列化风险

### 集成验证
- [ ] 与现有 Agent core 兼容
- [ ] HTTP 路由能正确调用
- [ ] 前端测试通过（如 ChatPanel.vue 能成功调用）

---

**文档完成时间**: 2026-07-26 18:00 UTC+8  
**下一步**: Rust 实现者阅读本文档并按实现检查清单逐项完成。若有歧义，请在实现前与协调者确认。
