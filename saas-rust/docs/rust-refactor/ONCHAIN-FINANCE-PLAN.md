# 链上财务功能完善方案

> 日期：2026-08-07 ｜ 分支：beta/rust-migration ｜ 开发环境：本地 WSL（领先服务器）
> 现状基准：git `c668503b`（本地=服务器已同步）

---

## 一、现状调查结论（已完成，非拍脑袋）

### 已实现且对齐 Python 的部分（无需重做）

| 层 | 组件 | 说明 |
|----|------|------|
| API | `routes/monitor.rs`（15+ 端点）| list/create/delete/remark/pnl_summary/export/transactions/year_summary/claim/tx_meta/batch/counterparty_aliases/user_queries —— 与 Python 全对齐 |
| API | `routes/finance.rs`（1001 行，9+ v1 端点）| summary/reconcile/categories/calculate_pnl/report/unreconciled/tokens/audit_logs/download_report + `/api/v1/monitors/finance/*` 兼容族 |
| Worker | `address_monitor_worker.rs`（508 行）| TRC20（TronGrid events）+ BSC（BscScan logs）扫描 → 解析 → **事务性入库（去重）→ job_queue 入队 alert → 更新游标**，完整对齐 Python chain_monitor.py |
| Worker | `monitor_worker.rs`（404 行）| 余额监控 + 价格告警 |
| Worker | `AddressMonitorAlertWorker` | 已注册（main.rs:164/168），alert 消息格式化（含测试）|
| 集成 | `trongrid.rs`（409 行）+ `bscscan.rs`（92 行）+ `binance.rs` | 链上数据源客户端 |
| DB | migrations 0004/0030/0031 | finance_ledgers / monitor_transactions（tenant key）/ monitor_alert_outbox |
| 前端 | 生产 Python OnChainFinancePanel.vue（1333 行）+ onChainFinance.ts store（API_BASE=`/api/v1/monitors/finance`）| 与 origin/master 一致（前端红线：不得改 UI 适配后端）|

### 真实差距（需要完善）

| # | 差距 | 类型 | 影响 |
|---|------|------|------|
| A | `finance_service.rs` / `monitor_service.rs` 是 1 行 stub，业务 SQL 散落在 routes 层 | 架构债务 | 逻辑难复用/难测试，代码重复 |
| B | 链上财务**报表导出**完整度未验证（report 生成、download_report 实际可下载性）| 功能风险 | 用户点导出可能失败 |
| C | 链上扫描/解析/对账的 **Rust 测试与 Python 行为一致性**未系统验证 | 质量风险 | 迁移正确性未锁定 |
| D | 本地验收未跑**真实链上数据**（需真实 TronGrid/BscScan key）| 验收缺口 | P-04~P-07 未闭环 |

---

## 二、完善方案（3 阶段，全部在本地 WSL 执行）

### 阶段 1：对齐验证 + 架构下沉（≈2-3 轮行动）

**目标**：锁死现有功能正确性，消除架构债务。

1. **链上财务回归测试**（本地）
   - 用 mock 链上数据（本地 mock-trongrid/bscscan 或脚本注入 monitor_transactions）验证：monitors CRUD → 扫描入库 → alert 入队 → finance summary/reconcile/pnl/report 全链路。
   - 验证 `/api/v1/monitors/finance/*` 与前端 onChainFinance.ts 全部方法一一对应（跑 beta-runtime-check 契约）。
2. **service 层下沉**
   - `finance_service.rs`：把 finance.rs 路由中的重复 SQL（summary/reconcile/categories/pnl/report）抽象为可复用 service 方法，路由只做编排。
   - `monitor_service.rs`：monitors CRUD + tx_meta + 别名 + 导出逻辑下沉。
   - 目的：消除 stub、可单测、为阶段 2 增强打基础。**不改变 API 契约**（前端零改动）。
3. **补测试**
   - 链上解析（parse_tron_transfers/parse_bsc_transfers）单测（已有部分，补边界）。
   - reconcile/pnl 计算单测（与 Python 结果对照）。

### 阶段 2：功能增强（按需，先做高价值）

| 增强 | 说明 | 优先级 |
|------|------|--------|
| 报表导出增强 | report 支持 PDF/CSV 下载（现状仅 JSON？需确认）、导出字段补齐 | 高 |
| 对账建议引擎 | 按 counterparty 聚合未核销流水，给出建议分类 | 中 |
| PnL 增强 | 持仓成本 / 已实现 vs 未实现盈亏拆分 | 中 |
| 余额快照 | 定时余额快照 + 余额变化趋势图数据 | 中 |

> 阶段 2 的每一项先出小方案，你确认后再做（避免过度设计）。

### 阶段 3：真机验收（链上数据闭环）

1. 配置真实 `TRONGRID_API_KEY` / `BSCSCAN_API_KEY`（写入本地环境或 DB secret）。
2. 注册 1-2 个真实监控地址（如用户自己的钱包），跑 `address_monitor_worker`。
3. 验证：真实转账 → 扫描 → 入库 → Telegram 告警 → finance 对账/PnL 展示 → 报表导出。
4. 覆盖 HANDOFF 中 P-04~P-07 的链上部分。

---

## 三、验收标准

- 阶段 1：beta-runtime-check 全绿（含新增 finance/monitor service 单测）；前端 102/102 契约不变。
- 阶段 2：按确认的增强项逐项验收。
- 阶段 3：真实链上转账触发告警 + 对账正确（人工验收）。

## 四、风险与依赖

- 真实链上 key 需用户提供（阶段 3）。
- 前端不得改动（origin/master 红线）——增强只做后端，UI 复用现有 OnChainFinancePanel。
- 阶段 1 的 service 下沉要小心不改路由行为（纯重构，跑回归确认）。

---
*本文档为方案设计，未改动代码。调查与方案已记录 WORKBUDDY-WORKLOG.md 行动 14。*
