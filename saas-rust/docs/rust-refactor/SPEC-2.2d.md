# 批次 2.2d 移植规格：确定性路由与电报证据格式化

**作者**: Claude Code (Haiku)  
**日期**: 2026-07-26  
**覆盖范围**: PORTING-13-COMMITS.md 项目 7、8、9，加项目 3（确定性格式化部分）  
**约束**: 零 LLM 依赖读取 Python codebase，无行号依赖（关键函数 ref）  

---

## 概述

本批次实现三大功能块，合计 6 个行为变更：

1. **确定性路由白名单扩展** (deb8eaf)：finance_query、detect_advertising、telegram_sdk_query 入兜底路由；__TOOL_CALL__ 容忍前导空白
2. **电报证据分层格式化** (1b109f5)：仅官方 API 验证的群计数入表格，DB 多出记录标记"未验证"且不计数；可删/不可删分组
3. **bound_chats 完整库存验证** (42bafb0)：SDK 验证上限从 10 提升至 50；Rust system_query 缺失 bound_chats，本规格补齐全设计
4. **确定性系统查询格式化** (915dcd6)：统一虚拟助手 UID 后的查询结果格式化（仅涉及输出字符串，迁移指引见 1.1）
5. **群 ID 数字优先级** (4c3f1dc)：显式数字 group_id 优先于群名解析，统一注入各分支

---

## 行为 1：确定性工具匹配与白名单扩展

### Python 源代码

**文件**: `services/ai_agent/core.py`

- **函数** `_match_tool_from_text`（行号 ~908）
  - 关键词列表：finance_query、detect_advertising、telegram_sdk_query 均有动词触发词
  - 评分逻辑：关键词命中 +8，工具名命中 +6，最低阈值 8（低于 16）
  - **白名单判定**（行号 ~997）：
    ```python
    if best_match in {
        "finance_query", "system_query", "telegram_sdk_query", "detect_advertising",
        "query_ad_clusters", "query_recent_ad_messages",
    } and best_score >= 8:
        return best_match
    return best_match if best_score >= 16 else None
    ```
  - **实际关键词表**：
    - `finance_query`: `["查财务", "财务摘要", "财务查询", "财务报表", "财务分析", "查账目", "查流水", "看账目", "看流水", "未核销", "待核销", "核销流水", "对账单", "对个账"]`
    - `detect_advertising`: `["检测广告", "广告检测", "这条是不是广告", "这段是不是广告", "鉴别这条", "检测这条消息"]`
    - `telegram_sdk_query`: `["机器人权限", "bot权限", "删除权限", "群权限", "频道权限", "机器人能不能删除", "为什么删除失败", "telegram sdk"]`

- **函数** `_build_fallback_args`（行号 ~1004）
  - finance_query 参数构造（行号 ~1017）
    ```python
    if "摘要" in text:
        return {"action": "summary"}
    if any(k in text for k in ("未核销", "待核销")):
        return {"action": "unreconciled"}
    if any(k in text for k in ("科目", "分类")):
        return {"action": "categories"}
    if any(k in text.lower() for k in ("trc20", "bsc")) and "测试" in text:
        return {"action": "onchain_api_test"}
    return {"action": "analysis"}
    ```
  - detect_advertising 参数：`{"text": text, "context_type": "private_message"}`
  - telegram_sdk_query 参数（行号 ~1050）
    ```python
    if tool_name == "telegram_sdk_query":
        group_match = re.search(r"(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
        if group_match:
            return {
                "operation": "chat_capabilities",
                "chat_id": int(group_match.group(1)),
                "include_admins": False,
            }
        return {
            "operation": "bound_chat_capabilities",
            "limit": 20,  # ← 后续 42bafb0 改为 50
            "include_admins": False,
        }
    ```

### Rust 实现目标

**目标文件**: `crates/agent/src/core.rs`

- **函数** `match_tool_from_text(&self, text: &str, tools: &[Value]) -> Option<String>`
  - 关键词 map 扩展 finance_query、detect_advertising、telegram_sdk_query
  - 白名单集合改为：`["finance_query", "system_query", "telegram_sdk_query", "detect_advertising", "query_ad_clusters", "query_recent_ad_messages"]`
  - 评分阈值：白名单工具 >= 8，其他 >= 16

- **函数** `build_fallback_args(&self, tool_name: &str, user_text: &str) -> Value`
  - finance_query：检查 "摘要"/"未核销"/"待核销"/"科目"/"分类"/"trc20"/"bsc" 关键词→相应 action
  - detect_advertising：返回 `{"text": text, "context_type": "private_message"}`
  - telegram_sdk_query：优先提取 group_id/chat_id；否则返回 `"operation": "bound_chat_capabilities", "limit": 20`

### 边界情况

1. **空白处理**：工具名完全匹配（不区分大小写）应加分
2. **重复工具匹配**：若多个工具关键词冲突，选择得分最高者；同分时保持确定性顺序
3. **finance 参数优先级**：摘要 > 未核销 > 科目 > 链测试 > 默认分析

---

## 行为 2：__TOOL_CALL__ 标记容忍前导空白

### Python 源代码

**文件**: `services/ai_agent/core.py`

- **函数** `_call_llm`（行号 ~1239）
  ```python
  # 尝试从返回文本中解析 function calling（显式标记模式）
  tool_calls = []
  stripped_text = text.strip() if isinstance(text, str) else text
  if isinstance(stripped_text, str) and stripped_text.startswith("__TOOL_CALL__"):
      try:
          tool_data = json.loads(stripped_text[len("__TOOL_CALL__"):])
          tool_calls = tool_data if isinstance(tool_data, list) else [tool_data]
      except Exception as e:
          logger.warning(f"Tool call JSON 解析失败，当作纯文本: {e}")
  ```
  - **关键点**：`text.strip()` 先执行，再检查 startswith

### Rust 实现目标

**目标文件**: `crates/agent/src/core.rs`

- **函数** `parse_llm_response(&response: &str) -> (String, Vec<ToolCall>)`
  - 先 `let stripped = response.trim();`
  - 然后检查 `if stripped.starts_with("__TOOL_CALL__")`
  - 提取 JSON：`stripped[len("__TOOL_CALL__"):].trim()`
  - 解析失败返回 `(response.to_string(), vec![])`

### 测试案例

| 名称 | 输入 | 期望行为 |
|------|------|---------|
| 无空白 | `__TOOL_CALL__[{...}]` | 解析工具调用 |
| 前导空白 | `  \n__TOOL_CALL__[{...}]` | 解析工具调用 |
| 尾部空白 | `__TOOL_CALL__[{...}]  ` | 解析工具调用 |
| JSON 失败 | `__TOOL_CALL__{invalid}` | 警告，作纯文本返回 |

---

## 行为 3：电报证据分层格式化（无 LLM）

### Python 源代码

**文件**: `services/ai_agent/core.py`

#### 3a. SDK 结果格式化

**函数** `_format_telegram_sdk_result`（行号 ~788）

```python
@classmethod
def _format_telegram_sdk_result(cls, result: Dict[str, Any]) -> str:
    if not result.get("success", True):
        return f"Telegram SDK 查询失败：{result.get('message') or '未知错误'}"
    verified, unavailable = cls._telegram_capability_rows(result)
    if not verified:
        return "Telegram 官方 API 没有验证到当前 Bot 可访问的群或频道。"
    
    deletable = []
    non_deletable = []
    for item in verified:
        permissions = item.get("bot_permissions") or {}
        status = str(item.get("bot_status") or permissions.get("status") or "unknown")
        can_delete = status == "creator" or (
            status == "administrator" and bool(permissions.get("can_delete_messages"))
        )
        label = f"{item.get('title') or '未命名'} ({item.get('chat_id')})"
        (deletable if can_delete else non_deletable).append(f"{label} [{status}]")
    
    lines = [
        f"Telegram 官方 API 已验证 {len(verified)} 个可访问聊天："
        f"{len(deletable)} 个可删除消息，{len(non_deletable)} 个不可删除。"
    ]
    if deletable:
        lines.append("可删除：" + "、".join(deletable))
    if non_deletable:
        lines.append("不可删除：" + "、".join(non_deletable))
    if unavailable:
        lines.append(f"另有 {len(unavailable)} 条数据库绑定记录未通过 Telegram 验证，不计入可用聊天。")
    return "\n".join(lines)
```

**输出范例**：
```
Telegram 官方 API 已验证 3 个可访问聊天：2 个可删除消息，1 个不可删除。
可删除：可删群 (-1001) [administrator]、创建者群 (-1002) [creator]
不可删除：受限群 (-1003) [restricted]
另有 1 条数据库绑定记录未通过 Telegram 验证，不计入可用聊天。
```

#### 3b. 结合验证的 bound_chats 格式化

**函数** `_format_verified_bound_chats`（行号 ~820）

```python
@classmethod
def _format_verified_bound_chats(cls, system_result: Dict[str, Any], sdk_result: Dict[str, Any]) -> str:
    verified, unavailable = cls._telegram_capability_rows(sdk_result)
    if not verified and not unavailable:
        return (
            cls._format_system_query_result(system_result)
            + "\nTelegram SDK 本次未返回可核验记录，以上数量仅来自数据库绑定表。"
        )
    groups = [item for item in verified if item.get("chat_type") in {"group", "supergroup"}]
    channels = [item for item in verified if item.get("chat_type") == "channel"]
    lines = [f"Telegram 官方 API 当前验证到群组 {len(groups)} 个，频道 {len(channels)} 个。"]
    if groups:
        lines.append("群组：")
        lines.extend(
            f"{index}. {item.get('title') or '未命名'} ({item.get('chat_id')})"
            for index, item in enumerate(groups, 1)
        )
    if channels:
        lines.append("频道：")
        lines.extend(
            f"{index}. {item.get('title') or '未命名'} ({item.get('chat_id')})"
            for index, item in enumerate(channels, 1)
        )
    db_data = system_result.get("data") or {}
    db_total = int(db_data.get("group_count") or 0) + int(db_data.get("channel_count") or 0)
    if unavailable or db_total > len(verified):
        lines.append(
            f"数据库绑定表共有 {db_total} 条群/频道记录，其中 {len(unavailable)} 条未通过官方验证，"
            "可能是测试、失效或迁移后的旧记录，未计入上述数量。"
        )
    return "\n".join(lines)
```

**输出范例**（assuming 2 verified groups, 1 verified channel, 1 unavailable in DB）：
```
Telegram 官方 API 当前验证到群组 2 个，频道 1 个。
群组：
1. 群A (-1001)
2. 群B (-1002)
频道：
1. 频道C (-2001)
数据库绑定表共有 4 条群/频道记录，其中 1 条未通过官方验证，可能是测试、失效或迁移后的旧记录，未计入上述数量。
```

#### 3c. 辅助函数

**函数** `_telegram_capability_rows`（行号 ~778）

```python
@staticmethod
def _telegram_capability_rows(result: Dict[str, Any]) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    data = result.get("data") or {}
    items = data.get("items") if isinstance(data, dict) else None
    if not isinstance(items, list):
        items = [data] if isinstance(data, dict) and data.get("chat_id") else []
    verified = [item for item in items if item.get("success") and item.get("chat_type")]
    unavailable = [item for item in items if item not in verified]
    return verified, unavailable
```

- **verified 条件**：`success=True` 且 `chat_type` 非空
- **unavailable**：其余项（包括 success=False 或缺 chat_type）

### Rust 实现目标

**目标文件**: `crates/agent/src/core.rs`

#### 3a 实现

- **函数** `format_telegram_sdk_result(result: &Value) -> String`
  - 校验 `result["success"]` 默认 true；失败返回错误文案
  - 调用 `telegram_capability_rows(&result)` 分离 verified/unavailable
  - verified 为空→返回 `"Telegram 官方 API 没有验证到当前 Bot 可访问的群或频道。"`
  - 遍历 verified，判断 `item.bot_status == "creator" || (item.bot_status == "administrator" && item.bot_permissions.can_delete_messages)` → deletable
  - 组装输出字符串（完全匹配 Python 格式）

#### 3b 实现

- **函数** `format_verified_bound_chats(system_result: &Value, sdk_result: &Value) -> String`
  - 调用 `telegram_capability_rows(&sdk_result)` 分离
  - verified 和 unavailable 都为空→拼接 `format_system_query_result(system_result) + "\nTelegram SDK 本次未返回可核验记录，以上数量仅来自数据库绑定表。"`
  - 否则分离 groups/channels（按 chat_type）
  - 格式化为序号列表（从 1 开始）
  - 对比 DB 总数与验证数，追加 "未验证" 声明

#### 3c 实现

- **函数** `telegram_capability_rows(result: &Value) -> (Vec<Value>, Vec<Value>)`
  - 提取 `result["data"]["items"]` 或 `result["data"]`
  - verified：`success=true` 且 `chat_type` 非 null
  - unavailable：其余

### 边界情况

1. **空值处理**：title 缺失→显示 `"未命名"`；chat_id 缺失→跳过该项
2. **状态判定**：
   - creator 总是可删
   - administrator 仅当 can_delete_messages=true 时可删
   - 其他状态（restricted, member 等）一律不可删
3. **DB vs 官方矛盾**：必须说明数字来源（"官方 API 验证" vs "仅数据库记录"）

---

## 行为 4：bound_chats 查询类型（完整设计）

### Python 源代码

**文件**: `services/ai_agent/tools.py`

**函数** `_query_bound_chats`（行号 ~1504）

```python
async def _query_bound_chats(self, tenant_id: Optional[str], limit: int) -> dict:
    groups = await database.get_tracked_groups_with_stats(tenant_id=tenant_id)
    targets = await database.get_broadcast_targets(enabled_only=False, tenant_id=tenant_id)

    tracked_by_id: Dict[int, Dict[str, Any]] = {}
    for group in groups:
        try:
            chat_id = int(group.get("group_id"))
        except Exception:
            continue
        tracked_by_id[chat_id] = self._serialize_chat_target(group, "supergroup")

    channels: List[Dict[str, Any]] = []
    broadcast_only_groups: List[Dict[str, Any]] = []
    for target in targets:
        try:
            chat_id = int(target.get("chat_id"))
        except Exception:
            continue
        chat_type = str(target.get("chat_type") or "")
        serialized = self._serialize_chat_target(target, chat_type)
        if chat_id in tracked_by_id:
            tracked_by_id[chat_id].update({
                "chat_type": chat_type or tracked_by_id[chat_id].get("chat_type"),
                "username": serialized.get("username") or tracked_by_id[chat_id].get("username"),
                "can_broadcast": serialized.get("can_broadcast"),
                "updated_at": serialized.get("updated_at") or tracked_by_id[chat_id].get("updated_at"),
            })
        elif chat_type == "channel":
            channels.append(serialized)
        else:
            serialized["source"] = "broadcast_targets_only"
            broadcast_only_groups.append(serialized)

    groups_only = list(tracked_by_id.values())
    all_targets = groups_only + channels + broadcast_only_groups

    return {
        "success": True,
        "data_complete": True,
        "evidence_count": len(all_targets),
        "data": {
            "group_count": len(groups_only),
            "channel_count": len(channels),
            "broadcast_only_group_count": len(broadcast_only_groups),
            "total_count": len(all_targets),
            "groups": groups_only[:limit],
            "channels": channels[:limit],
            "broadcast_only_groups": broadcast_only_groups[:limit],
        },
        "count": len(all_targets),
        "message": (
            f"机器人绑定聊天：群组 {len(groups_only)} 个，频道 {len(channels)} 个"
            + (f"，另有仅广播目标群 {len(broadcast_only_groups)} 个" if broadcast_only_groups else "")
        ),
    }
```

- **数据分层**：
  1. tracked_groups → groups（优先级最高）
  2. broadcast_targets 中 chat_type="channel" → channels
  3. broadcast_targets 中其他 + 未在 tracked 中 → broadcast_only_groups
- **合并逻辑**：若 broadcast_targets 中的 chat_id 在 tracked 中，更新其元数据（chat_type、username、can_broadcast、updated_at）

### Python 兜底参数（build_fallback_args）

**文件**: `services/ai_agent/core.py`，行号 ~1109

```python
if any(k in text for k in (
    "有多少群", "有几个群", "几个群", "有多少频道", "有几个频道", "几个频道",
    "绑定群", "绑定频道", "绑定群聊", "群列表", "频道列表", "机器人绑定",
    "查看群聊", "查询群聊", "我的群聊",
    "我有多少群", "我有几个群", "我有多少频道", "我有几个频道",
)):
    return {"query_type": "bound_chats", "limit": 20}
```

### 确定性预执行与 SDK 组合（core.rs）

**文件**: `services/ai_agent/core.py`，`_build_preflight_tool_calls`，行号 ~1202

```python
needs_sdk_visibility = query_type in {"group_messages", "channel_messages", "group_ad_messages", "bound_chats"}
sdk_available = any(t.get("function", {}).get("name") == "telegram_sdk_query" for t in tools)
if needs_sdk_visibility and sdk_available:
    sdk_args = {
        "operation": "bot_visibility_status" if query_type != "bound_chats" else "bound_chat_capabilities",
        "limit": 50 if query_type == "bound_chats" else 10,  # ← 42bafb0 关键变更
        "include_admins": False,
    }
    if args.get("group_id"):
        sdk_args["chat_id"] = args["group_id"]
    calls.append({
        "function": {
            "name": "telegram_sdk_query",
            "arguments": json.dumps(sdk_args, ensure_ascii=False),
        }
    })
```

- **限制**：bound_chats 查询 SDK 上限 50，其他 10

### Rust 实现目标

**目标文件**: `crates/agent/src/tools/system_query.rs`

#### 4a. system_query 工具扩展

- **参数 enum** 增加 `QueryType::BoundChats`
- **schema 更新**：
  ```rust
  "enum": [
      "stats_overview",
      "active_groups",
      "monitors_list",
      "recent_senders",
      "recent_messages",
      "group_ad_messages",
      "user_stats",
      "potential_users",
      "group_messages",
      "channel_messages",
      "bound_chats"  // ← 新增
  ]
  ```

#### 4b. 执行分支

```rust
"bound_chats" => self.query_bound_chats(&pool, tenant_id, limit).await
```

#### 4c. 新增函数

**函数** `query_bound_chats(&self, pool: &PgPool, tenant_id: &str, limit: i32) -> Result<Value, Error>`

- 查询 `tracked_groups`（精确租户 scope）→ groups_only
  - 选择列：group_id, group_title, chat_type=supergroup, username, updated_at, 其他聊天元数据
- 查询 `broadcast_targets`（enabled_only=false, 精确租户 scope）
  - 分离：
    - chat_type='channel' → channels
    - 其他且不在 groups_only 中 → broadcast_only_groups
- 返回结构（JSON）：
  ```json
  {
    "success": true,
    "data_complete": true,
    "evidence_count": <总数>,
    "data": {
      "group_count": <groups_only 数>,
      "channel_count": <channels 数>,
      "broadcast_only_group_count": <broadcast_only 数>,
      "total_count": <总数>,
      "groups": [...],
      "channels": [...],
      "broadcast_only_groups": [...]
    },
    "message": "机器人绑定聊天：群组 X 个，频道 Y 个[，另有仅广播目标群 Z 个]"
  }
  ```

#### 4d. 确定性预执行补充

**目标文件**: `crates/agent/src/core.rs`，`build_preflight_tool_calls`

```rust
// 当 system_query query_type="bound_chats" 时，自动追加 telegram_sdk_query
if query_type == "bound_chats" && sdk_available {
    sdk_args.insert("operation", "bound_chat_capabilities");
    sdk_args.insert("limit", 50);  // ← 42bafb0 关键
    sdk_args.insert("include_admins", false);
}
```

### 边界情况

1. **chat_id 解析**：若 int() 失败，跳过该项
2. **broadcast_targets 既有又新**：先检查是否在 tracked_by_id；若是，merge；否则分类
3. **空集合**：group/channel/broadcast 任何为空都要输出，但 message 中会有条件分支
4. **limit 应用**：limit 仅应用于 groups/channels/broadcast_only_groups 中各取前 N，总数不受限

---

## 行为 5：确定性系统查询格式化（915dcd6）

### Python 源代码

**文件**: `services/ai_agent/core.py`

**函数** `_format_system_query_result`（行号 ~691）

```python
@staticmethod
def _format_system_query_result(result: Dict[str, Any]) -> str:
    if not result.get("success", True):
        return f"系统查询失败：{result.get('message') or '未知错误'}"
    data = result.get("data")
    if isinstance(data, dict) and "total_count" in data:
        groups = data.get("groups") or []
        channels = data.get("channels") or []
        broadcast_only = data.get("broadcast_only_groups") or []
        lines = [
            f"当前绑定群组 {int(data.get('group_count') or 0)} 个，频道 {int(data.get('channel_count') or 0)} 个。"
        ]
        if broadcast_only:
            lines.append(f"另有仅广播目标群 {int(data.get('broadcast_only_group_count') or 0)} 个。")
        if groups:
            lines.append("群组：" + "、".join(
                f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                for item in groups[:20]
            ))
        if channels:
            lines.append("频道：" + "、".join(
                f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                for item in channels[:20]
            ))
        if broadcast_only:
            lines.append("仅广播目标群：" + "、".join(
                f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                for item in broadcast_only[:20]
            ))
        return "\n".join(lines)

    if isinstance(data, list):
        evidence_count = int(result.get("evidence_count") or result.get("count") or len(data))
        if evidence_count <= 0:
            return result.get("limitation") or result.get("message") or "当前没有可用的已落库数据。"
        previews = []
        for item in data[:10]:
            if not isinstance(item, dict):
                continue
            title = item.get("group_title") or item.get("title") or item.get("username") or item.get("user_id") or "记录"
            content = str(item.get("content") or "").strip().replace("\n", " ")[:80]
            previews.append(f"{title}: {content}" if content else str(title))
        heading = result.get("message") or f"查询到 {evidence_count} 条记录"
        return heading + ("\n" + "\n".join(f"- {item}" for item in previews) if previews else "")

    if isinstance(data, dict):
        summary_items = [
            f"{key}={value}" for key, value in data.items()
            if isinstance(value, (str, int, float, bool))
        ]
        return (result.get("message") or "系统查询完成") + ("\n" + "，".join(summary_items) if summary_items else "")
    return result.get("message") or "系统查询完成。"
```

### 输出范例

**Case 1**: data 为 bound_chats 结构（has total_count）
```
当前绑定群组 5 个，频道 2 个。
另有仅广播目标群 1 个。
群组：群A (-1001)、群B (-1002)、群C (-1003)、群D (-1004)、群E (-1005)
频道：频道1 (-2001)、频道2 (-2002)
仅广播目标群：测试群 (-3001)
```

**Case 2**: data 为 list（如消息列表）
```
查询到 5 条记录
- 群A: 你好，这是一条消息
- 用户123: 这是另一条消息的内容摘要（最多 80 字）
```

**Case 3**: data 为 dict（无 total_count）
```
系统查询完成
group_count=5，channel_count=2，user_count=10
```

**Case 4**: 失败或空值
```
系统查询失败：数据库连接超时
当前没有可用的已落库数据。
系统查询完成。
```

### Rust 实现目标

**目标文件**: `crates/agent/src/core.rs`

**函数** `format_system_query_result(result: &Value) -> String`

- 检查 success，默认 true；false 返回错误文案
- 按 data 类型分支：
  1. **有 total_count**：格式化群/频道/广播列表
  2. **列表**：逐项预览（前 10 项），每项最多 80 字符内容
  3. **dict 无 total_count**：遍历标量字段，逗号连接
  4. **默认**：返回 message 或 `"系统查询完成。"`

- **字符串格式**（完全匹配 Python）：
  - `f"当前绑定群组 {group_count} 个，频道 {channel_count} 个。"`
  - `f"另有仅广播目标群 {broadcast_only_group_count} 个。"` （仅当 > 0）
  - `"群组：" + "、".join(...)`
  - `"频道：" + "、".join(...)`
  - `"仅广播目标群：" + "、".join(...)`

### 边界情况

1. **字段缺失**：title 或 chat_title 缺失→ `"未命名"`；都缺→使用 user_id 或 `"记录"`
2. **内容换行**：content 中 `\n` 替换为 ` ` 空格
3. **截断**：内容最多 80 字符；groups/channels/broadcast 各自列表截断 20 项
4. **列表为空**：若 groups 为 [], "群组：" 这行不输出

---

## 行为 6：数字 group_id 优先级（4c3f1dc）

### Python 源代码

**文件**: `services/ai_agent/core.py`

**函数** `_build_fallback_args`（行号 ~1088）

```python
if tool_name == "system_query":
    lower = text.lower()
    group_match = re.search(r"(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
    group_id = int(group_match.group(1)) if group_match else None
    group_name = "" if group_id is not None else self._extract_group_name(text)  # ← 关键

    def add_group_scope(args: dict) -> dict:
        if group_id is not None:
            args["group_id"] = group_id
        elif group_name:
            args["group_name"] = group_name
        return args
```

- **关键**：若 group_id 非 None，则不设置 group_name；否则尝试从文本提取
- **兜底函数** `add_group_scope` 应用于各分支

### Rust 实现目标

**目标文件**: `crates/agent/src/core.rs`

**函数** `build_fallback_args(&self, tool_name: &str, user_text: &str) -> Value`

在 system_query 分支中：

```rust
let group_match: Option<i64> = extract_group_id_from_text(text);  // regex: group_id|chat_id|群|群组.*(-?\d{5,})
let group_name = if group_match.is_some() {
    // 若已有数字，不提取名字
    ""
} else {
    extract_group_name_from_text(text)
};

// 后续在各分支中
if let Some(gid) = group_match {
    args["group_id"] = gid;
} else if !group_name.is_empty() {
    args["group_name"] = group_name;
}
```

### 边界情况

1. **正则匹配**：`group_id|chat_id|群|群组` 这些关键词可选，但 `-?\d{5,}` 必须满足（最少 5 位数字，可负）
2. **优先级清晰**：显式 group_id > 提取的 group_name > 都没有（不设置任何 scope）
3. **跨群误删防护**：若用户说 "删除广告"，系统必须拿到明确 group_id，否则拒绝（在确定性预执行中体现）

---

## 集成点：预执行与确定性格式化流程

### 调用链（core.rs 对标 Python process_message）

```
process_message(user_text, ctx)
  ↓
build_preflight_tool_calls(user_text)  // 行为 1、6 应用于匹配
  ├─ match_tool_from_text(user_text) → matched_tool
  ├─ build_fallback_args(matched_tool, user_text) → args（包含 group_id 优先级）
  └─ 针对 finance_query / detect_advertising / telegram_sdk_query / system_query 
     + query_recent_ad_messages / query_ad_clusters
     预执行 → tool_results
  ↓
确定性格式化（行为 2、3、5）
  ├─ if system_query + telegram_sdk_query + query_type == "bound_chats"
  │  → format_verified_bound_chats(system_result, sdk_result)
  ├─ else if system_query
  │  → format_system_query_result(system_result)
  ├─ else if telegram_sdk_query
  │  → format_telegram_sdk_result(sdk_result)
  └─ ...其他确定性工具格式化...
  ↓
parse_llm_response(llm_answer)  // 行为 2：先 trim 再检查 __TOOL_CALL__
  └─ 若无工具调用，返回 llm_answer 作最终回复
```

---

## 测试用例清单

### 行为 1：确定性路由

| ID | 名称 | 输入文本 | 期望工具 | 期望参数 | 备注 |
|---|---|---|---|---|---|
| 1.1 | finance_query 摘要 | "查一下财务摘要" | finance_query | `{"action": "summary"}` | 关键词命中 |
| 1.2 | finance_query 未核销 | "看看未核销的" | finance_query | `{"action": "unreconciled"}` | 优先级 |
| 1.3 | detect_advertising 简短 | "检测广告" | detect_advertising | `{"text": "检测广告", "context_type": "private_message"}` | 关键词 ≥8 |
| 1.4 | telegram_sdk_query 权限 | "机器人有删除权限吗" | telegram_sdk_query | `{"operation": "bound_chat_capabilities", "limit": 20, ...}` | 无群 ID |
| 1.5 | telegram_sdk_query + group | "检查群 -1001 的删除权限" | telegram_sdk_query | `{"operation": "chat_capabilities", "chat_id": -1001, ...}` | 有群 ID 优先 |
| 1.6 | 兜底失败（短文本） | "删" | 无 | 无 | 长度 < 4 时不匹配 |

### 行为 2：__TOOL_CALL__ 容忍空白

| ID | 名称 | 输入 | 解析成功 | 备注 |
|---|---|---|---|---|
| 2.1 | 无空白 | `__TOOL_CALL__[{"name": "test"}]` | ✓ | 标准格式 |
| 2.2 | 前导换行 | `\n__TOOL_CALL__[...]` | ✓ | strip 后 |
| 2.3 | 前导空格 | `  __TOOL_CALL__[...]` | ✓ | strip 后 |
| 2.4 | JSON 失败 | `__TOOL_CALL__{invalid` | ✗ | 警告返回原文 |

### 行为 3：证据分层

| ID | 名称 | SDK verified | SDK unavailable | 期望文案 | 备注 |
|---|---|---|---|---|---|
| 3.1 | 纯官方验证 | 3 (2可删, 1不可) | 0 | "API 已验证 3 个...2 个可删，1 个不可删" | 不提及 DB |
| 3.2 | 官方 + DB 多 | 2 | 1 | "另有 1 条数据库...未通过...验证" | 分层说明 |
| 3.3 | 官方无验证 | 0 | 1 | "Telegram 官方 API 没有验证到...若 system_query 有结果则合并 | 降级到 system |
| 3.4 | bound_chats 组合 | system + sdk | 见范例 | "Telegram 官方 API...群组 X...频道 Y...数据库...Z 条未验证" | 序号列表 |

### 行为 4：bound_chats

| ID | 名称 | groups | channels | broadcast_only | 期望 count | 期望 message | 备注 |
|---|---|---|---|---|---|---|---|
| 4.1 | 仅群 | 5 | 0 | 0 | 5 | "群组 5 个，频道 0 个" | 不提及仅广播 |
| 4.2 | 混合 | 3 | 2 | 1 | 6 | "群组 3 个，频道 2 个，另有仅广播...1 个" | 三层都输出 |
| 4.3 | 全空 | 0 | 0 | 0 | 0 | "群组 0 个，频道 0 个" | 允许空 |
| 4.4 | SDK limit | - | - | - | 50 limit | 预执行时 telegram_sdk_query limit=50 | 42bafb0 |

### 行为 5：system_query 格式化

| ID | 名称 | data 结构 | 期望输出 | 备注 |
|---|---|---|---|---|
| 5.1 | dict total_count | `{total_count, groups: [...], channels: [...]}` | 多行格式 | 见范例 |
| 5.2 | list 消息 | `[{title, content}, ...]` | "查询到 N 条记录\n- 标题: 内容" | 预览 10 条 |
| 5.3 | dict 无 total_count | `{group_count: 5, ...}` | "系统查询完成\ngroup_count=5,..." | 标量键值 |
| 5.4 | 失败 | `success: false` | "系统查询失败：..." | 取 message |
| 5.5 | 空 | evidence_count ≤ 0 | "当前没有可用...已落库数据" | limitation 或默认文案 |

### 行为 6：group_id 优先级

| ID | 名称 | 输入文本 | 提取 group_id | 提取 group_name | 最终参数 | 备注 |
|---|---|---|---|---|---|---|
| 6.1 | 显式 ID | "查群 -1001 的消息" | -1001 | 不提取 | `{group_id: -1001}` | 数字优先 |
| 6.2 | 纯名字 | "查「测试群」的广告" | None | "测试群" | `{group_name: "测试群"}` | 无 ID 时提取 |
| 6.3 | ID + 名字 | "群 -1001 叫「测试群」" | -1001 | 不提取 | `{group_id: -1001}` | ID 优先忽视名字 |
| 6.4 | 都无 | "查最近消息" | None | "" | 无 scope 参数 | 类型决定行为 |

---

## 实现检查清单

### 文件修改

- [ ] `crates/agent/src/core.rs`
  - [ ] `match_tool_from_text()` 关键词表扩展（finance/detect/telegram）
  - [ ] 白名单评分阈值改为 >=8
  - [ ] `build_fallback_args()` finance 参数构造
  - [ ] `build_fallback_args()` detect 参数
  - [ ] `build_fallback_args()` telegram 参数
  - [ ] `build_fallback_args()` system_query group_id 优先级逻辑
  - [ ] `parse_llm_response()` 改为 `response.trim()` 后 startswith
  - [ ] `format_system_query_result()`（五分支逻辑）
  - [ ] `format_telegram_sdk_result()`
  - [ ] `format_verified_bound_chats()`
  - [ ] `telegram_capability_rows()` 辅助函数
  - [ ] `build_preflight_tool_calls()` bound_chats SDK 限制改 50

- [ ] `crates/agent/src/tools/system_query.rs`
  - [ ] 参数 schema 添加 "bound_chats"
  - [ ] execute 分支添加 "bound_chats"
  - [ ] `query_bound_chats()` 新函数（含三层数据聚合）

### 单元测试

- [ ] `tests/agent/test_tool_matching.rs`
  - [ ] finance_query 关键词匹配 6 个
  - [ ] detect_advertising 关键词 4 个
  - [ ] telegram_sdk_query 关键词 5 个
  - [ ] 白名单阈值 8 vs 非白名单 16
  - [ ] finance 参数优先级（摘要 > 未核销 > 科目）

- [ ] `tests/agent/test_tool_call_parsing.rs`
  - [ ] 无空白正常解析
  - [ ] 前导空白解析成功
  - [ ] JSON 失败返回原文

- [ ] `tests/agent/test_evidence_formatting.rs`
  - [ ] telegram_sdk_result 正常格式
  - [ ] 可删/不可删分组
  - [ ] unavailable 计数和文案
  - [ ] verified_bound_chats 组合输出
  - [ ] 无验证数据降级

- [ ] `tests/tools/test_system_query.rs`
  - [ ] bound_chats 三层聚合（groups, channels, broadcast_only）
  - [ ] 字段缺失（title, chat_id）处理
  - [ ] 限制应用（list 20, preview 10）
  - [ ] 群 ID int parse 失败跳过

- [ ] `tests/agent/test_system_query_formatting.rs`
  - [ ] 输出格式匹配 Python 范例（字符分隔符 "、"）
  - [ ] 广播群条件分支（数 > 0 时才输出行）
  - [ ] 空 groups/channels/broadcast 处理
  - [ ] 内容截断 80 字
  - [ ] 列表截断 20 项

- [ ] `tests/agent/test_group_id_precedence.rs`
  - [ ] 显式 ID 不提取名字
  - [ ] 无 ID 时提取名字
  - [ ] 都无时无 scope 参数

- [ ] `tests/integration/test_bound_chats_with_sdk.rs`
  - [ ] preflight system_query + telegram_sdk_query
  - [ ] bound_chats SDK limit = 50
  - [ ] verified_bound_chats 输出正确序号列表

### 集成测试

- [ ] `tests/integration/test_deterministic_routing.rs`
  - [ ] 完整流程：finance 摘要查询 → preflight → 确定性格式化 → 无 LLM 回复
  - [ ] detect_advertising 无 LLM 回复
  - [ ] system_query + telegram_sdk_query 组合格式化

- [ ] 数据库迁移（如涉及）
  - [ ] 无 schema 变更（本批纯代码实现）

### 代码审查要点

- [ ] 关键词列表与 Python 完全一致（中文字符逐字验证）
- [ ] 输出字符串中文标点符号一致（"、"而非","；"。"而非"."）
- [ ] 条件分支逻辑（can_delete 判断, status 等）与 Python 等价
- [ ] 错误处理（parse 失败、空值、类型不匹配）安全降级
- [ ] limit/截断应用正确（20 vs 10 vs 50）
- [ ] 租户 scope 强制（system_query 返回必须过滤）

---

## 已知限制与后续任务

1. **虚拟助手 UID 统一**（915dcd6 part）：本规格不覆盖，参考任务 1.1 inbox 过滤与迁移
2. **ad_text 指纹库**（f4ec872）：归一化逻辑需位级与 Python MD5 一致，本批不含
3. **确认流程**（519921b）：confirm_and_execute 回复摘要应单独实现，本批不含
4. **n8n 适配**：本规格为纯后端，n8n 节点升级由其他任务完成

---

## 参考资源

- Python 源：`/home/ubuntu/tgbot/services/ai_agent/core.py` (L 691-850, L 908-1220)
- Python 源：`/home/ubuntu/tgbot/services/ai_agent/tools.py` (L 1504-1760)
- Python 测试：`/home/ubuntu/tgbot/tests/test_ai_agent_llm_and_ad_delete.py` (finance_summary, telegram_permissions, bound_chat 测试)
- Rust 骨架：`/home/ubuntu/tgbot-rust-beta/saas-rust/crates/agent/src/core.rs`
- Rust 工具：`/home/ubuntu/tgbot-rust-beta/saas-rust/crates/agent/src/tools/system_query.rs`

---

END OF SPEC-2.2d
