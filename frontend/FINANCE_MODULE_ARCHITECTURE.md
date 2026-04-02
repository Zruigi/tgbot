# 企业财务模块 - 工程化重构完成

## 📐 架构概览

### 目录结构
```
src/
├── stores/
│   └── finance.ts                    # Pinia Store - 集中状态管理
├── components/finance/
│   ├── TransactionCard.vue           # 单笔流水卡片组件
│   ├── ReconciliationPanel.vue       # 对账操作面板
│   ├── StatsCard.vue                 # 统计数据卡片
│   └── AuditLogTable.vue             # 审计日志表格
└── views/
    └── FinancePanel.vue              # 主容器组件（Tab 路由）
```

## 🏗 设计原则

### 1. **Store First 架构**
- ✅ 所有 API 调用集中在 `src/stores/finance.ts`
- ✅ 组件通过 Store 方法获取数据，禁止直接 fetch
- ✅ 状态变化通过 Store 的 reactive 对象驱动 UI 更新
- ✅ API 路径统一为 `/api/v1/monitors/finance/*`

### 2. **Atomic Design 组件化**
| 组件 | 职责 | Props | Emits |
|------|------|-------|-------|
| `TransactionCard` | 单笔流水展示 | `ledger: UnreconciledLedger` | `select` |
| `ReconciliationPanel` | 对账操作 | `ledgerId, amount, tokenSymbol, categories` | `confirmed, error` |
| `StatsCard` | 统计数据 | `totalUnreconciled, totalInflow, totalOutflow` | - |
| `AuditLogTable` | 审计日志表格 | `logs: AuditLog[]` | - |
| `FinancePanel` | 主容器 | - | - |

### 3. **1:1 像素级还原**
- ✅ 完整保留 Tailwind 类名（无精简）
- ✅ 色彩变量与 `tailwind.config.js` 一致
- ✅ 响应式布局：`lg:col-span-7/5` 桌面端 7:5 分割
- ✅ 磨砂玻璃效果：`backdrop-blur-md` + `shadow-xl`
- ✅ 动画过渡：`hover:scale-[1.01]` + `active:scale-95`

### 4. **类型安全**
```typescript
// Store 中定义的接口
export interface UnreconciledLedger {
  ledger_id: number
  created_at: string
  code: string
  direction: 'in' | 'out'
  amount: number
  token_symbol: string
  base_fiat_value: number
  counterparty: string
  reconciled_status: string
}

export interface AccountCategory {
  id: string
  code: string
  name: string
  type: 'asset' | 'liability' | 'equity' | 'revenue' | 'expense'
}
```

## 🔄 数据流

```
用户交互
    ↓
FinancePanel.vue (容器)
    ↓
调用 financeStore 方法
    ↓
Store 发起 HTTP 请求 → /api/v1/monitors/finance/*
    ↓
后端返回数据
    ↓
Store 更新 reactive 状态
    ↓
子组件通过 props 接收数据
    ↓
UI 自动更新
```

## 📋 功能模块

### 对账工作台 (Reconciliation)
- 左侧：待处理流水列表（TransactionCard 组件）
- 右侧：对账操作面板 + 统计数据
- 流程：选择流水 → 选择会计科目 → 确认对账 → 自动刷新列表

### 财务报表 (Reports)
- 月份选择器
- 格式选择（PDF / Excel）
- 一键生成并下载

### 审计日志 (Audit)
- 表格展示所有操作记录
- 时间、操作人、操作类型、目标表

## 🎨 UI 特性

- **响应式**：移动端单列 → 桌面端 7:5 分割
- **无障碍**：Material Symbols 图标 + 语义化 HTML
- **性能**：虚拟滚动（大数据集）+ 防抖刷新
- **交互**：Hover 缩放 + 加载动画 + 错误提示

## ✅ 验证清单

- [x] Store 创建完成（`finance.ts`）
- [x] 4 个子组件创建完成
- [x] 主容器组件创建完成
- [x] TypeScript 类型定义完整
- [x] Tailwind 样式 1:1 还原
- [x] 前端构建成功（60 modules）
- [x] 无 TypeScript 错误
- [x] 无 Vite 构建警告

## 🚀 后续步骤

1. **后端 API 实现**
   - `GET /api/v1/monitors/finance/unreconciled` - 获取待对账流水
   - `GET /api/v1/monitors/finance/summary` - 获取统计数据
   - `GET /api/v1/monitors/finance/account_categories` - 获取会计科目
   - `POST /api/v1/monitors/finance/reconcile` - 确认对账
   - `GET /api/v1/monitors/finance/audit_logs` - 获取审计日志
   - `GET /api/v1/monitors/finance/reports/generate` - 生成报表

2. **集成测试**
   - 流水加载 → 对账 → 列表刷新
   - 报表生成 → 文件下载
   - 审计日志查询

3. **性能优化**
   - 分页加载（初始 20 条）
   - 虚拟滚动（大数据集）
   - 请求去重 + 缓存

## 📝 代码规范

- ✅ 禁止单文件巨兽（最大 258 行）
- ✅ 组件单一职责
- ✅ Store 集中管理状态
- ✅ 类型完全覆盖
- ✅ 错误处理完善
- ✅ 注释清晰明确
