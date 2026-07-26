# 13 提交移植规格(9ca636d..332ba4c → Rust beta)

> 2026-07-26 由分析 agent 产出、Fable 审定。作为阶段 2.2 各移植任务的规格来源。
> 总结论:Rust 侧对本系列覆盖率为零(关键符号全 crates 无匹配);13/13 提交含 A 必移植项,C 已覆盖 0 项。
> 绝不能丢的安全闸:服务端重查 refs(ea76526)、仅删已确认簇 409 闸(f08faed)、禁止 LLM 直呼删除工具(c816441)、快照删除 ≤50/15min TTL/逐条复核/AD_DELETE_ENABLED 默认关(31d4607)、治理默认 dry-run + 双开关默认关(332ba4c)。

## 逐提交清单(旧→新)

1. **31d4607 make agent ad deletion tool-driven and bounded 【A】** — 快照式删除:QueryRecentAdMessagesTool/DeleteRecentAdMessagesTool,只删后端确定性快照(≤50 条、15 分钟失效、逐条回库复核 msg/group/tg_msg/user + is_advertising=1 + ad_score≥40、排除管理员/bot/频道、AD_DELETE_ENABLED 总闸、ad_action_status='deleting' 抢占防重、审计落库);core 确定性预路由 `_prepare_recent_ad_deletion`,查询结果不经 LLM 格式化;意图路由 LLM JSON mode + 温度0 + 15s 超时 + 输入截断 500 字 + 真实 chat_id 覆盖 LLM 提取值防注入。
   移植:crates/agent/src/tools/ 新增两工具并入 safety.rs DANGEROUS_ACTIONS;migrations 补 group_messages 的 is_deleted/deleted_at(以及 pre-fork 就缺的 ad_action_status/ad_cluster_id/ad_checked_at);routes/ai.rs::ai_chat 重写为结构化意图路由(强类型解析,解析失败拒绝执行工具)。

2. **519921b report confirmed agent actions unambiguously 【A】** — 确认执行后回复只描述本次执行结果(区分部分执行/未执行),不拼接旧"请确认"文本。
   移植:crates/agent/src/core.rs `confirm_and_execute`(~478 行仍是修复前拼接写法)改为仅返回执行结果摘要,has_failure && action_taken → "部分执行"。

3. **915dcd6 unify agent contact and enforce tool routes 【A+B】** — A:AI_ASSISTANT_USER_ID/LEGACY_AI_ASSISTANT_USER_IDS 统一虚拟助手 UID,inbox 永远排除虚拟助手;删除意图加否定/疑问守卫("不要删除/为什么删除"不触发删除流);system_query 确定性格式化。B:ChatPanel.vue(前端,不移植)。
   移植:/api/users 过滤(与任务 1.1 重合,复核即可);crates/migration/migrate_messages.rs 迁移时 legacy UID 900000001 → -99999 且不导入其 inbox 行;删除预路由带否定/疑问守卫。

4. **f4ec872 harden agent ad cleanup pipeline 【A+B】** — A:ad_text 统一归一化/哈希;group_messages 加 ad_text_hash 列+索引;新表 ad_text_fingerprints/ad_sender_profiles(租户级已确认广告知识库,命中免 LLM);快照前逐群校验 bot 删除权限;include_sender_messages 仅显式指令时扩展;GET /api/agent/tools/manifest 与 /api/agent/ad-fingerprints GET/POST/DELETE。B:n8n-nodes JS(不移植)。
   移植:migrations 补列+两表;crates/services 新增 ad_text 模块(**MD5(归一化文本) 必须与 Python 位级一致**,否则指纹库迁移后失效);agent_tools.rs 实现 manifest + fingerprints 三端点(租户 scope 强制)。

5. **ea76526 validate ad clusters before deletion 【A·安全闸】** — 永不信任客户端/LLM 提交的 message_refs:服务端按 cluster_id 重查(查不到→404);单批≤50;逐 chat 校验 bot 身份与 can_delete_messages;缺 sender 校验 fail-closed。
   移植:tool_delete_ad_cluster 入参只收 cluster_id/window_minutes/dry_run,refs 全部服务端重查。

6. **f08faed require confirmed ad clusters for deletion 【A·安全闸】** — 仅 llm_status="ad" 或含 is_advertising=1 消息的簇可删,候选簇一律 409。
   移植:cluster 查询带 confirmed_for_deletion 推导字段,删除路径二次校验。

7. **deb8eaf route sdk and finance queries deterministically 【A】** — finance/detect_advertising/telegram_sdk_query 入确定性兜底路由白名单,finance 结果代码格式化,__TOOL_CALL__ 容忍前导空白。
   移植:core.rs match_tool_from_text/build_fallback_args 扩展白名单;parse_llm_response 先 trim。

8. **1b109f5 format telegram evidence without llm 【A】** — SDK 组合结果纯代码格式化:只把 Telegram 官方 API 验证到的群计数,DB 多出的记录标注"未验证,不计入",可删/不可删分组。**证据分层:DB 记录 ≠ Telegram 可见,不得混报。**

9. **42bafb0 verify complete bound chat inventory 【A小】** — bound_chats SDK 核验 limit 10→50。Rust system_query 目前无 bound_chats 类型,补齐时直接按 50 实现。

10. **c816441 resolve contextual agent commands 【A·安全闸】** — (a) 工具循环内硬拦截 LLM 自发 delete_recent_ad_messages 调用(safety 检查之前);(b) 会话取消词表("取消/不要了/停止"清空 pending 不执行);(c) 简短跟进指令("删除""第N个")基于上一轮已验证回复解析。

11. **4c3f1dc preserve resolved group ids in agent queries 【A小】** — 兜底参数显式数字 group_id 优先于群名解析,统一注入 system_query 各分支。

12. **afe8a31 keep active group context in agent sessions 【A小】** — 会话 metadata 记 active_group_id/last_ad_query_group_id;"这个群"/裸"删除"跟进继承上次查询群作用域(跨群误删防护)。

13. **332ba4c add full-history ad governance scans 【A+B·最大项】** — A:ad_governance 服务(6 张新表:ad_group_policies/ad_scan_jobs/ad_governance_clusters/+cluster_messages/+actions/ad_warning_dedup;扫描 job 状态机首扫强制 dry-run→确认才 activate;精确+模板相似度聚类;LLM 只对簇代表文本做是/否分类,family_hash 白名单 + confidence≥0.65,不得生成删除指令;动作分层 warn/delete/mute;AD_AUTO_ENFORCE_ENABLED+AD_DELETE_ENABLED 双闸;action_key 幂等审计;警告去重;逐群锁+租户 scope);/api/agent/ad-scan/* 端点族;chat 响应扩展 ui_actions/job(messages.agent_ui_json 列);消息摄取采集 media_unique_id 并调度实时扫描。B:ChatPanel.vue(不移植)。
   移植:migrations 6 表+2 列;crates/services/ad_governance 新服务;routes 新增 ad-scan 族;telegram_webhook.rs 摄取处(~945 行 INSERT group_messages)补 media_unique_id/ad_text_hash 与实时扫描钩子。

## 建议移植顺序(2.2 拆任务依据)

1. Schema 批:31d4607/f4ec872/332ba4c 的全部 migrations(新迁移文件,不改旧文件)
2. 删除安全闸批:ea76526 → f08faed → 31d4607 快照工具 → c816441(a) 硬拦截
3. 会话层批:519921b/c816441(b)(c)/afe8a31/4c3f1dc(确认/取消/上下文/群作用域)
4. 确定性路由与格式化批:915dcd6/deb8eaf/1b109f5/42bafb0
5. 治理子系统批:332ba4c(最大,最后做)

注:任务 1.2(agent_tools stub 实装)按当前生产 Python(已含全系列修复)对齐语义,应天然覆盖 5/6 的大部分——2.2 开工前先 diff 复核避免重复实现。
