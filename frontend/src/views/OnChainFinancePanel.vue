<template>
  <div class="uc-page bg-surface">
    <header class="uc-header">
      <div class="uc-header-wrap">
        <div>
          <h2 class="uc-title">
            <span class="material-symbols-outlined text-primary">account_balance_wallet</span>
            链上财务管理
          </h2>
          <p class="uc-subtitle">统一链上流水、地址监听、对账备注与统计视图</p>
        </div>
        <div class="uc-actions">
          <button class="uc-btn uc-btn-ghost" @click="handleRefresh">
            <span class="material-symbols-outlined text-base">refresh</span>
            刷新
          </button>
          <button class="uc-btn uc-btn-primary" @click="showAddMonitorModal = true">
            <span class="material-symbols-outlined text-base">add</span>
            新增监听
          </button>
        </div>
      </div>
    </header>

    <main class="uc-main" ref="canvasRef">
      <div class="uc-main-wrap">
        <section class="uc-card">
          <div class="uc-grid uc-grid-cols-6">
            <label class="uc-field">
              <span class="uc-label">时间范围</span>
              <select v-model="store.filters.period" class="uc-input" @change="onPeriodChange">
                <option value="day">今日</option>
                <option value="week">本周</option>
                <option value="month">本月</option>
                <option value="custom">自定义</option>
              </select>
            </label>
            <label class="uc-field" v-if="store.filters.period === 'custom'">
              <span class="uc-label">开始日期</span>
              <input v-model="store.filters.start" class="uc-input" type="date" />
            </label>
            <label class="uc-field" v-if="store.filters.period === 'custom'">
              <span class="uc-label">结束日期</span>
              <input v-model="store.filters.end" class="uc-input" type="date" />
            </label>
            <label class="uc-field">
              <span class="uc-label">币种</span>
              <select v-model="store.filters.token_symbol" class="uc-input">
                <option value="ALL">全部币种</option>
                <option v-for="token in store.availableTokens" :key="token" :value="token">{{ token }}</option>
              </select>
            </label>
            <label class="uc-field">
              <span class="uc-label">网络</span>
              <select v-model="store.filters.network" class="uc-input">
                <option value="all">全部网络</option>
                <option value="trc20">TRC20</option>
                <option value="bsc">BSC</option>
              </select>
            </label>
            <label class="uc-field">
              <span class="uc-label">流水过滤</span>
              <select v-model="annotatedFilter" class="uc-input">
                <option :value="false">全部流水</option>
                <option :value="true">仅已备注</option>
              </select>
            </label>
            <label class="uc-field">
              <span class="uc-label">去杂过滤</span>
              <select v-model="store.filters.exclude_dust" class="uc-input">
                <option :value="false">关闭</option>
                <option :value="true">过滤转入小额</option>
              </select>
            </label>
            <label class="uc-field" v-if="store.filters.exclude_dust">
              <span class="uc-label">去杂阈值 (USDT)</span>
              <input v-model.number="store.filters.dust_threshold" class="uc-input" type="number" min="0" step="0.01" />
            </label>
          </div>
          <div v-if="store.timelineLock" class="timeline-lock-pill">
            <span class="material-symbols-outlined text-[14px]">lock</span>
            已锁定时间轴：{{ store.timelineLock.start }} ~ {{ store.timelineLock.end }}（仅本次登录）
            <button class="timeline-lock-export" @click="handleExportLockedTimelineCsv">导出锁定时间CSV</button>
          </div>
          <div class="finance-filter-actions">
            <button class="uc-btn uc-btn-primary" @click="handleApplyFilters">应用筛选</button>
            <button
              v-if="store.filters.period === 'custom'"
              class="uc-btn uc-btn-ghost"
              @click="handleLockTimeline"
            >
              锁定本次时间轴
            </button>
            <button v-if="store.timelineLock" class="uc-btn uc-btn-ghost" @click="handleUnlockTimeline">解除时间轴锁定</button>
            <button class="uc-btn uc-btn-ghost" @click="resetFilters">重置</button>
          </div>
        </section>

        <section class="uc-grid uc-grid-cols-5">
          <div class="uc-stat">
            <div class="uc-stat-label">监听地址</div>
            <div class="uc-stat-value">{{ store.monitors.length }}</div>
          </div>
          <div class="uc-stat">
            <div class="uc-stat-label">期间流水</div>
            <div class="uc-stat-value">{{ store.summary.tx_count }}</div>
          </div>
          <div class="uc-stat">
            <div class="uc-stat-label">总入金</div>
            <div class="uc-stat-value text-emerald-600">${{ formatNum(store.summary.total_inflow) }}</div>
          </div>
          <div class="uc-stat">
            <div class="uc-stat-label">总出金</div>
            <div class="uc-stat-value text-rose-600">${{ formatNum(store.summary.total_outflow) }}</div>
          </div>
          <div class="uc-stat">
            <div class="uc-stat-label">对账成功率</div>
            <div class="uc-stat-value">{{ formatNum(store.summary.success_rate) }}%</div>
          </div>
        </section>

        <section class="uc-card">
          <div class="uc-tabs">
            <button
              v-for="tab in tabs"
              :key="tab.id"
              @click="store.activeTab = tab.id"
              :class="['uc-tab', store.activeTab === tab.id ? 'active' : '']"
            >
              {{ tab.label }}
            </button>
          </div>
        </section>

        <section v-if="store.activeTab === 'monitors'" class="uc-card">
          <div class="finance-table-wrap">
            <table class="finance-table">
              <thead>
                <tr>
                  <th>ID</th>
                  <th>地址</th>
                  <th>网络</th>
                  <th>地址备注</th>
                  <th class="text-right">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="m in store.monitors" :key="m.monitor_id">
                  <td class="font-bold">#{{ m.monitor_id }}</td>
                  <td class="font-mono text-xs break-all">{{ m.address }}</td>
                  <td>
                    <span class="finance-network-pill">{{ m.network }}</span>
                  </td>
                  <td>
                    <input
                      class="uc-input"
                      :value="monitorRemarks[m.monitor_id] ?? m.remark ?? ''"
                      @input="onMonitorRemarkInput(m.monitor_id, $event)"
                      placeholder="填写地址备注"
                    />
                  </td>
                  <td class="text-right">
                    <div class="finance-row-actions">
                      <button class="uc-btn uc-btn-ghost" @click="viewMonitorTransactions(m.monitor_id)">查看流水</button>
                      <button class="uc-btn uc-btn-ghost" @click="saveMonitorRemark(m.monitor_id)">保存备注</button>
                      <button class="uc-btn uc-btn-danger" @click="store.deleteMonitor(m.monitor_id)">删除</button>
                    </div>
                  </td>
                </tr>
                <tr v-if="store.monitors.length === 0">
                  <td class="uc-empty" colspan="5">暂无监听地址</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-else-if="store.activeTab === 'reconciliation'" class="finance-reconcile-grid">
          <div class="uc-card">
            <h3 class="uc-card-title">流水记录</h3>
            <p class="uc-card-subtitle">点击流水卡片即可编辑订单备注并执行对账</p>
            <p v-if="focusedMonitorText" class="uc-card-subtitle">当前地址：{{ focusedMonitorText }}</p>
            <div class="reconcile-toolbar">
              <div class="reconcile-toolbar-left">
                <span>已选 {{ selectedTxIds.length }} 笔</span>
                <span class="text-outline-variant">共 {{ store.transactions.length }} 笔</span>
              </div>
              <div class="reconcile-toolbar-right">
                <button class="uc-btn uc-btn-ghost" @click="selectAllTransactionsInView" :disabled="store.transactions.length === 0">全选</button>
                <button class="uc-btn uc-btn-ghost" @click="clearSelectedTransactions" :disabled="selectedTxIds.length === 0">清空选择</button>
              </div>
            </div>

            <div v-if="store.isLoadingTransactions" class="uc-empty">加载中...</div>

            <div v-else class="tx-list">
              <button
                v-for="tx in store.transactions"
                :key="tx.tx_id"
                class="tx-card"
                :class="store.selectedTransaction?.tx_id === tx.tx_id ? 'active' : ''"
                @click="selectTx(tx)"
              >
                <div class="tx-head">
                  <div>
                    <div class="tx-time">{{ formatTs(tx.tx_timestamp) }}</div>
                    <div class="tx-id">{{ tx.tx_id }}</div>
                  </div>
                  <div class="tx-head-right">
                    <label class="tx-check" @click.stop>
                      <input
                        type="checkbox"
                        :checked="selectedTxIds.includes(tx.tx_id)"
                        @change="toggleTxSelection(tx.tx_id)"
                      />
                    </label>
                    <span
                      class="tx-direction"
                      :class="tx.direction === 'in' ? 'in' : 'out'"
                    >
                      {{ tx.direction === 'in' ? '转入' : '转出' }}
                    </span>
                  </div>
                </div>
                <div class="tx-body">
                  <span class="font-bold">{{ tx.amount }} {{ tx.token_symbol }}</span>
                  <span>{{ tx.network }}</span>
                  <span class="break-all">{{ tx.address }}</span>
                </div>
                <div class="tx-meta">
                  对手方：{{ tx.counterparty || '-' }}
                  <span v-if="tx.counterparty_label">（{{ tx.counterparty_label }}）</span>
                  <span v-else-if="(tx.counterparty_tx_count || 0) >= 2">（重复 {{ tx.counterparty_tx_count }} 笔）</span>
                </div>
                <div class="tx-tags">
                  <span v-if="tx.income_reason" class="tx-tag">入账备注: {{ tx.income_reason }}</span>
                  <span v-if="tx.expense_reason" class="tx-tag">出账备注: {{ tx.expense_reason }}</span>
                  <span v-if="tx.payer" class="tx-tag">付款人: {{ tx.payer }}</span>
                  <span v-if="tx.payee" class="tx-tag">收款人: {{ tx.payee }}</span>
                </div>
              </button>
            </div>

            <div v-if="!store.isLoadingTransactions && store.transactions.length === 0" class="uc-empty">
              当前筛选条件下暂无流水
            </div>
          </div>

          <div class="finance-side-stack">
            <section class="uc-card">
              <h3 class="uc-card-title">订单备注 / 对账操作</h3>
              <div v-if="store.selectedTransaction" class="finance-form-stack">
                <div>
                  <div class="uc-label">交易ID</div>
                  <div class="tx-id break-all">{{ store.selectedTransaction.tx_id }}</div>
                </div>
                <label class="uc-field">
                  <span class="uc-label">对手方地址标签</span>
                  <input v-model="counterpartyLabelInput" class="uc-input" placeholder="例如：张三、渠道A、客服退款地址" />
                </label>

                <label class="uc-field">
                  <span class="uc-label">入账备注</span>
                  <input v-model="txMetaForm.income_reason" class="uc-input" placeholder="例如：客户补款" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">出账备注</span>
                  <input v-model="txMetaForm.expense_reason" class="uc-input" placeholder="例如：渠道扣费" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">付款人</span>
                  <input v-model="txMetaForm.payer" class="uc-input" placeholder="付款方名称" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">收款人</span>
                  <input v-model="txMetaForm.payee" class="uc-input" placeholder="收款方名称" />
                </label>

                <button class="uc-btn uc-btn-primary" @click="saveTxMeta">保存订单备注</button>

                <div class="finance-divider"></div>

                <label class="uc-field">
                  <span class="uc-label">会计科目</span>
                  <select v-model="store.selectedAccountCode" class="uc-input">
                    <option value="">选择科目</option>
                    <option value="1001">1001 - 现金</option>
                    <option value="1002">1002 - 银行存款</option>
                    <option value="1010">1010 - 应收账款</option>
                    <option value="1020">1020 - 库存资产</option>
                  </select>
                </label>
                <button class="uc-btn uc-btn-ghost" @click="handleReconcile">执行对账</button>
              </div>
              <div v-else class="uc-empty">选择一笔流水后可编辑备注与执行对账。</div>
            </section>

            <section class="uc-card">
              <h3 class="uc-card-title">批量备注</h3>
              <p class="uc-card-subtitle">对选中的流水批量更新备注信息</p>
              <div class="finance-form-stack">
                <label class="uc-field">
                  <span class="uc-label">入账备注</span>
                  <input v-model="batchMetaForm.income_reason" class="uc-input" placeholder="批量入账备注（可留空）" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">出账备注</span>
                  <input v-model="batchMetaForm.expense_reason" class="uc-input" placeholder="批量出账备注（可留空）" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">付款人</span>
                  <input v-model="batchMetaForm.payer" class="uc-input" placeholder="批量付款人（可留空）" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">收款人</span>
                  <input v-model="batchMetaForm.payee" class="uc-input" placeholder="批量收款人（可留空）" />
                </label>
                <button class="uc-btn uc-btn-primary" :disabled="selectedTxIds.length === 0 || batchSaving" @click="saveBatchMeta">
                  {{ batchSaving ? '保存中...' : `批量保存（${selectedTxIds.length}）` }}
                </button>
              </div>
            </section>

            <section class="uc-card">
              <h3 class="uc-card-title">币种统计</h3>
              <div class="token-summary-list">
                <div v-for="item in store.summary.by_token" :key="item.token_symbol + item.network" class="token-summary-item">
                  <div class="token-summary-head">
                    <span class="font-bold">{{ item.token_symbol }} / {{ item.network }}</span>
                    <span>{{ item.tx_count }} 笔</span>
                  </div>
                  <div class="token-summary-desc">
                    入 {{ formatNum(item.inflow) }} · 出 {{ formatNum(item.outflow) }} · 净 {{ formatNum(item.profit) }}
                  </div>
                </div>
                <div v-if="store.summary.by_token.length === 0" class="uc-empty">暂无统计</div>
              </div>
            </section>
          </div>
        </section>

        <section v-else-if="store.activeTab === 'reports'" class="uc-card">
          <h3 class="uc-card-title">报表中心</h3>
          <p class="uc-card-subtitle">生成月度财务报表，或直接导出当前筛选结果为 CSV。</p>
          <div class="reports-grid">
            <label class="uc-field">
              <span class="uc-label">报表月份</span>
              <input v-model="reportMonth" type="month" class="uc-input" />
            </label>
            <label class="uc-field">
              <span class="uc-label">报表格式</span>
              <select v-model="reportFormat" class="uc-input">
                <option value="pdf">PDF</option>
                <option value="excel">Excel</option>
              </select>
            </label>
          </div>
          <div class="finance-filter-actions">
            <button class="uc-btn uc-btn-primary" :disabled="store.isGeneratingReport" @click="handleGenerateReport">
              {{ store.isGeneratingReport ? '生成中...' : '下载月度报表' }}
            </button>
            <button class="uc-btn uc-btn-ghost" @click="handleExportCsv">导出当前筛选CSV</button>
            <button v-if="store.timelineLock" class="uc-btn uc-btn-ghost" @click="handleExportLockedTimelineCsv">导出锁定时间CSV</button>
          </div>
          <div class="report-note">
            当前筛选：{{ periodLabel }} · {{ store.filters.network === 'all' ? '全部网络' : store.filters.network.toUpperCase() }} · {{ store.filters.token_symbol === 'ALL' ? '全部币种' : store.filters.token_symbol }}
          </div>
          <div v-if="store.timelineLock" class="report-note">
            锁定时间导出：{{ lockedTimelineLabel }}
          </div>
        </section>

        <section v-else class="uc-card">
          <h3 class="uc-card-title">审计日志</h3>
          <p class="uc-card-subtitle">查看对账与财务动作记录，支持按天数和动作筛选。</p>
          <div class="audit-filters">
            <label class="uc-field">
              <span class="uc-label">时间范围</span>
              <select v-model="auditFilters.days" class="uc-input">
                <option :value="7">近7天</option>
                <option :value="30">近30天</option>
                <option :value="90">近90天</option>
              </select>
            </label>
            <label class="uc-field">
              <span class="uc-label">动作</span>
              <select v-model="auditFilters.action" class="uc-input">
                <option value="">全部动作</option>
                <option value="CREATE">CREATE</option>
                <option value="RECONCILE">RECONCILE</option>
                <option value="REVERSE">REVERSE</option>
                <option value="UPDATE">UPDATE</option>
                <option value="DELETE">DELETE</option>
              </select>
            </label>
            <label class="uc-field">
              <span class="uc-label">目标表</span>
              <select v-model="auditFilters.target_table" class="uc-input">
                <option value="">全部目标</option>
                <option value="finance_ledgers">finance_ledgers</option>
                <option value="monitor_transactions">monitor_transactions</option>
                <option value="address_monitors">address_monitors</option>
              </select>
            </label>
          </div>
          <div class="finance-filter-actions">
            <button class="uc-btn uc-btn-primary" :disabled="store.isLoadingAuditLogs" @click="handleLoadAuditLogs">
              {{ store.isLoadingAuditLogs ? '加载中...' : '刷新审计日志' }}
            </button>
          </div>
          <div class="audit-table-wrap">
            <table class="audit-table" v-if="store.auditLogs.length > 0">
              <thead>
                <tr>
                  <th>时间</th>
                  <th>动作</th>
                  <th>目标</th>
                  <th>详情</th>
                  <th>IP</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="log in store.auditLogs" :key="log.audit_id">
                  <td>{{ formatAuditTime(log.timestamp) }}</td>
                  <td><span class="audit-action">{{ log.action }}</span></td>
                  <td>{{ log.target_table }} <span v-if="log.target_id">#{{ log.target_id }}</span></td>
                  <td class="audit-reason">{{ log.reason || '-' }}</td>
                  <td>{{ log.ip_address || '-' }}</td>
                </tr>
              </tbody>
            </table>
            <div v-else class="uc-empty">暂无审计日志</div>
          </div>
        </section>
      </div>
    </main>

    <transition name="fade">
      <div v-if="showAddMonitorModal" class="uc-modal-overlay">
        <div class="uc-modal-box">
          <div class="finance-modal-head">
            <h3 class="uc-card-title">新增监听地址</h3>
            <button class="uc-btn uc-btn-ghost" @click="showAddMonitorModal = false">关闭</button>
          </div>
          <div class="finance-modal-form">
            <input v-model="newMonitor.user_id" class="uc-input" placeholder="用户 ID（可选）" type="number" />
            <input v-model="newMonitor.address" class="uc-input" placeholder="钱包地址" />
            <select v-model="newMonitor.network" class="uc-input">
              <option value="trc20">TRC20</option>
              <option value="bsc">BSC</option>
            </select>
          </div>
          <div class="uc-actions">
            <button class="uc-btn uc-btn-ghost" @click="showAddMonitorModal = false">取消</button>
            <button class="uc-btn uc-btn-primary" @click="handleAddMonitor">添加</button>
          </div>
        </div>
      </div>
    </transition>

    <transition name="fade">
      <div v-if="store.error" class="uc-toast">
        {{ store.error }}
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import type { OnChainTransaction } from '@/stores/onChainFinance'
import { useOnChainFinanceStore } from '@/stores/onChainFinance'

const store = useOnChainFinanceStore()
const canvasRef = ref<HTMLElement | null>(null)
const showAddMonitorModal = ref(false)
const annotatedFilter = ref(false)
const selectedTxIds = ref<string[]>([])
const batchSaving = ref(false)
const counterpartyLabelInput = ref('')
const reportMonth = ref(new Date().toISOString().slice(0, 7))
const reportFormat = ref<'pdf' | 'excel'>('pdf')
const auditFilters = reactive({
  days: 30,
  action: '',
  target_table: '',
})

const newMonitor = reactive({
  user_id: '',
  address: '',
  network: 'trc20',
})

const monitorRemarks = reactive<Record<number, string>>({})
const txMetaForm = reactive({
  income_reason: '',
  expense_reason: '',
  payer: '',
  payee: '',
})
const batchMetaForm = reactive({
  income_reason: '',
  expense_reason: '',
  payer: '',
  payee: '',
})

const tabs = [
  { id: 'monitors', label: '地址监听' },
  { id: 'reconciliation', label: '流水与备注' },
  { id: 'reports', label: '报表' },
  { id: 'audit', label: '审计' },
] as const

const periodLabel = computed(() => {
  if (store.timelineLock) {
    return `${store.timelineLock.start} ~ ${store.timelineLock.end}`
  }
  if (store.filters.period === 'custom') {
    return `${store.filters.start || '-'} ~ ${store.filters.end || '-'}`
  }
  if (store.filters.period === 'day') return '今日'
  if (store.filters.period === 'week') return '本周'
  return '本月'
})

const focusedMonitorText = computed(() => {
  if (store.focusedMonitorId === undefined) return ''
  const monitor = store.monitors.find((m) => m.monitor_id === store.focusedMonitorId)
  if (!monitor) return `#${store.focusedMonitorId}`
  return `${monitor.remark || '-'} (${monitor.address})`
})

const lockedTimelineLabel = computed(() => {
  if (!store.timelineLock) return ''
  return `${store.timelineLock.start} ~ ${store.timelineLock.end}`
})

watch(
  () => store.filters.annotated_only,
  (v) => {
    annotatedFilter.value = v
  },
  { immediate: true }
)

watch(annotatedFilter, (v) => {
  store.filters.annotated_only = v
})

watch(
  () => store.transactions.map((tx) => String(tx.tx_id)).join(','),
  () => {
    const valid = new Set(store.transactions.map((tx) => String(tx.tx_id)))
    selectedTxIds.value = selectedTxIds.value.filter((id) => valid.has(id))
  }
)

function formatNum(v: number | string) {
  const n = Number(v || 0)
  if (!Number.isFinite(n)) return '0'
  return n.toFixed(2)
}

function formatTs(ts?: number) {
  if (!ts) return '-'
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return '-'
  return d.toLocaleString('zh-CN')
}

function onPeriodChange() {
  store.setPeriod(store.filters.period)
}

function resetFilters() {
  store.clearTimelineLock()
  store.setFocusedMonitor(undefined)
  store.setPeriod('month')
  store.filters.token_symbol = 'ALL'
  store.filters.network = 'all'
  store.filters.annotated_only = false
  store.filters.exclude_dust = false
  store.filters.dust_threshold = 1
  store.filters.limit = 300
  void handleApplyFilters()
}

async function handleApplyFilters() {
  selectedTxIds.value = []
  const needSync = !!store.timelineLock || store.filters.period === 'custom'
  if (needSync) {
    await store.loadTransactions(undefined, { syncChain: true })
    await Promise.all([store.loadSummary(), store.loadMonitors()])
  } else {
    await Promise.all([store.loadSummary(), store.loadTransactions(), store.loadMonitors()])
  }
}

async function handleRefresh() {
  const tasks: Promise<any>[] = [store.refreshAll()]
  if (store.activeTab === 'audit') {
    tasks.push(handleLoadAuditLogs())
  }
  await Promise.all(tasks)
}

function onMonitorRemarkInput(monitorId: number, e: Event) {
  const v = (e.target as HTMLInputElement).value
  monitorRemarks[monitorId] = v
}

async function saveMonitorRemark(monitorId: number) {
  const monitor = store.monitors.find((m) => m.monitor_id === monitorId)
  if (!monitor) return
  const remark = (monitorRemarks[monitorId] ?? monitor.remark ?? '').trim()
  await store.updateMonitorRemark(monitorId, remark)
}

async function viewMonitorTransactions(monitorId: number) {
  store.activeTab = 'reconciliation'
  store.setFocusedMonitor(monitorId)
  selectedTxIds.value = []
  await store.loadTransactions(monitorId, { syncChain: !!store.timelineLock || store.filters.period === 'custom' })
  await store.loadSummary()
}

function selectTx(tx: OnChainTransaction) {
  store.selectTransaction(tx)
  counterpartyLabelInput.value = tx.counterparty_label || ''
  txMetaForm.income_reason = tx.income_reason || ''
  txMetaForm.expense_reason = tx.expense_reason || ''
  txMetaForm.payer = tx.payer || ''
  txMetaForm.payee = tx.payee || ''
}

function toggleTxSelection(txId: string) {
  const id = String(txId)
  const idx = selectedTxIds.value.indexOf(id)
  if (idx >= 0) selectedTxIds.value.splice(idx, 1)
  else selectedTxIds.value.push(id)
}

function selectAllTransactionsInView() {
  selectedTxIds.value = store.transactions.map((tx) => String(tx.tx_id))
}

function clearSelectedTransactions() {
  selectedTxIds.value = []
}

async function saveTxMeta() {
  const tx = store.selectedTransaction
  if (!tx) return
  await store.updateTransactionMeta(tx.tx_id, {
    ...txMetaForm,
    counterparty: tx.counterparty,
    counterparty_label: counterpartyLabelInput.value.trim() || undefined,
    network: tx.network,
  })
}

async function handleReconcile() {
  const tx = store.selectedTransaction
  if (!tx || !store.selectedAccountCode) return
  await store.reconcileTransaction(tx.ledger_id, store.selectedAccountCode, Number(tx.amount || 0))
}

async function saveBatchMeta() {
  if (selectedTxIds.value.length === 0) return
  batchSaving.value = true
  try {
    const payload = selectedTxIds.value.map((txId) => ({
      tx_id: txId,
      income_reason: batchMetaForm.income_reason,
      expense_reason: batchMetaForm.expense_reason,
      payer: batchMetaForm.payer,
      payee: batchMetaForm.payee,
    }))
    const updated = await store.batchUpdateTransactionMeta(payload)
    if (updated > 0) {
      selectedTxIds.value = []
    }
  } finally {
    batchSaving.value = false
  }
}

async function handleGenerateReport() {
  if (!reportMonth.value) {
    store.error = '请选择报表月份'
    return
  }
  await store.generateReport(reportMonth.value, reportFormat.value)
}

async function handleLockTimeline() {
  if (store.filters.period !== 'custom' || !store.filters.start || !store.filters.end) {
    store.error = '请先选择自定义时间范围后再锁定'
    return
  }
  const start = store.filters.start
  const end = store.filters.end
  if (!window.confirm(`锁定时间轴为 ${start} ~ ${end} ？\\n仅本次登录有效。`)) return
  store.lockTimeline(start, end)
  await handleApplyFilters()
}

function handleUnlockTimeline() {
  store.clearTimelineLock()
}

function handleExportCsv() {
  store.exportTransactionsCsv()
}

function handleExportLockedTimelineCsv() {
  if (!store.timelineLock) {
    store.error = '当前没有已锁定的时间轴'
    return
  }
  store.exportTransactionsCsv({
    filenamePrefix: `链上流水_锁定时间_${store.timelineLock.start}_${store.timelineLock.end}`,
    includeTimeline: store.timelineLock,
  })
}

async function handleLoadAuditLogs() {
  await store.loadAuditLogs({
    days: auditFilters.days,
    action: auditFilters.action || undefined,
    target_table: auditFilters.target_table || undefined,
  })
}

function formatAuditTime(ts?: string) {
  if (!ts) return '-'
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ts
  return d.toLocaleString('zh-CN')
}

async function handleAddMonitor() {
  if (!newMonitor.address) {
    store.error = '请填写钱包地址'
    return
  }
  const ok = await store.addMonitor(Number(newMonitor.user_id || 0), newMonitor.address, newMonitor.network)
  if (ok) {
    showAddMonitorModal.value = false
    newMonitor.user_id = ''
    newMonitor.address = ''
    newMonitor.network = 'trc20'
  }
}

onMounted(async () => {
  void canvasRef.value
  await Promise.all([store.refreshAll(), store.loadAuditLogs({ days: auditFilters.days })])
})

watch(
  () => store.activeTab,
  async (tab) => {
    if (tab === 'audit' && store.auditLogs.length === 0) {
      await handleLoadAuditLogs()
    }
  }
)
</script>

<style scoped>
.finance-filter-actions {
  margin-top: 10px;
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
}

.timeline-lock-pill {
  margin-top: 10px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  font-size: 12px;
  color: #0f172a;
  border-radius: 999px;
  padding: 6px 10px;
  background: rgb(250 204 21 / 0.22);
}

.dark .timeline-lock-pill {
  color: #fde68a;
  background: rgb(234 179 8 / 0.22);
}

.timeline-lock-export {
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  font-weight: 700;
  cursor: pointer;
  padding: 0 0 0 2px;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.timeline-lock-export:hover {
  opacity: 0.88;
}

.finance-table-wrap {
  border: 1px solid rgb(148 163 184 / 0.2);
  border-radius: 12px;
  overflow-x: auto;
  overscroll-behavior-x: contain;
  background: rgb(248 250 252 / 0.6);
}

.dark .finance-table-wrap {
  border-color: rgb(73 106 148 / 0.28);
  background: rgb(8 17 31 / 0.62);
}

.finance-table {
  width: 100%;
  min-width: 700px;
  border-collapse: collapse;
  font-size: 13px;
}

.finance-table thead th {
  text-align: left;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: #64748b;
  background: rgb(248 250 252 / 0.75);
  border-bottom: 1px solid rgb(148 163 184 / 0.2);
  padding: 10px 10px;
}

.dark .finance-table thead th {
  color: #adc3e0;
  background: rgb(12 32 56 / 0.62);
  border-bottom-color: rgb(73 106 148 / 0.3);
}

.finance-table tbody td {
  border-bottom: 1px solid rgb(148 163 184 / 0.16);
  padding: 10px 10px;
}

.dark .finance-table tbody td {
  border-bottom-color: rgb(73 106 148 / 0.22);
}

.finance-network-pill {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 4px 9px;
  font-size: 11px;
  font-weight: 800;
  letter-spacing: 0.03em;
  color: #0f172a;
  background: rgb(36 161 222 / 0.2);
}

.dark .finance-network-pill {
  color: #dbeafe;
  background: rgb(36 161 222 / 0.28);
}

.finance-row-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
}

.finance-reconcile-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 360px;
  gap: 12px;
  min-width: 0;
}

.finance-side-stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.tx-list {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.reconcile-toolbar {
  margin-top: 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

.reconcile-toolbar-left {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: #64748b;
}

.dark .reconcile-toolbar-left {
  color: #adc3e0;
}

.reconcile-toolbar-right {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.tx-card {
  width: 100%;
  border: 1px solid rgb(148 163 184 / 0.2);
  border-radius: 14px;
  background: rgb(248 250 252 / 0.72);
  padding: 12px;
  text-align: left;
  cursor: pointer;
  transition: border-color 0.18s ease, box-shadow 0.18s ease, transform 0.16s ease;
  min-width: 0;
}

.tx-card:hover {
  border-color: rgb(36 161 222 / 0.42);
}

.tx-card.active {
  border-color: rgb(36 161 222 / 0.56);
  box-shadow: 0 0 0 3px rgb(36 161 222 / 0.12);
}

.dark .tx-card {
  border-color: rgb(73 106 148 / 0.28);
  background: rgb(8 17 31 / 0.7);
}

.tx-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}

.tx-head > div {
  min-width: 0;
}

.tx-head-right {
  display: flex;
  align-items: center;
  gap: 6px;
}

.tx-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: 8px;
  border: 1px solid rgb(148 163 184 / 0.28);
  background: rgb(248 250 252 / 0.84);
}

.tx-check input {
  width: 14px;
  height: 14px;
}

.dark .tx-check {
  border-color: rgb(73 106 148 / 0.35);
  background: rgb(8 17 31 / 0.75);
}

.tx-time {
  font-size: 11px;
  color: #64748b;
}

.dark .tx-time {
  color: #adc3e0;
}

.tx-id {
  margin-top: 2px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.3;
  color: #0f172a;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.dark .tx-id {
  color: #dbeafe;
}

.tx-direction {
  border-radius: 999px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 800;
}

.tx-direction.in {
  background: rgb(16 185 129 / 0.16);
  color: rgb(6 95 70);
}

.tx-direction.out {
  background: rgb(244 63 94 / 0.16);
  color: rgb(159 18 57);
}

.dark .tx-direction.in {
  background: rgb(16 185 129 / 0.24);
  color: rgb(167 243 208);
}

.dark .tx-direction.out {
  background: rgb(244 63 94 / 0.24);
  color: rgb(254 205 211);
}

.tx-body {
  margin-top: 9px;
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  font-size: 12px;
  color: #334155;
}

.dark .tx-body {
  color: #bcd0ec;
}

.tx-meta {
  margin-top: 8px;
  font-size: 12px;
  color: #64748b;
  overflow-wrap: anywhere;
}

.dark .tx-meta {
  color: #adc3e0;
}

.tx-tags {
  margin-top: 8px;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.tx-tag {
  border-radius: 999px;
  padding: 4px 8px;
  font-size: 11px;
  color: #334155;
  background: rgb(148 163 184 / 0.18);
  max-width: 100%;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.dark .tx-tag {
  color: #dbeafe;
  background: rgb(73 106 148 / 0.26);
}

.finance-form-stack {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.finance-divider {
  height: 1px;
  background: rgb(148 163 184 / 0.24);
}

.dark .finance-divider {
  background: rgb(73 106 148 / 0.3);
}

.token-summary-list {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.token-summary-item {
  border-radius: 12px;
  border: 1px solid rgb(148 163 184 / 0.2);
  background: rgb(248 250 252 / 0.68);
  padding: 9px 10px;
}

.dark .token-summary-item {
  border-color: rgb(73 106 148 / 0.26);
  background: rgb(8 17 31 / 0.66);
}

.token-summary-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
  min-width: 0;
}

.token-summary-desc {
  margin-top: 5px;
  font-size: 11px;
  color: #64748b;
  overflow-wrap: anywhere;
}

.dark .token-summary-desc {
  color: #adc3e0;
}

.finance-modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 10px;
}

.finance-modal-form {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 10px;
}

.reports-grid,
.audit-filters {
  margin-top: 10px;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.audit-filters {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.report-note {
  margin-top: 8px;
  font-size: 12px;
  color: #64748b;
}

.dark .report-note {
  color: #adc3e0;
}

.audit-table-wrap {
  margin-top: 10px;
  border: 1px solid rgb(148 163 184 / 0.2);
  border-radius: 12px;
  overflow: auto;
  background: rgb(248 250 252 / 0.65);
}

.dark .audit-table-wrap {
  border-color: rgb(73 106 148 / 0.28);
  background: rgb(8 17 31 / 0.62);
}

.audit-table {
  width: 100%;
  min-width: 760px;
  border-collapse: collapse;
  font-size: 12px;
}

.audit-table thead th {
  text-align: left;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: #64748b;
  border-bottom: 1px solid rgb(148 163 184 / 0.2);
  padding: 8px 9px;
  background: rgb(248 250 252 / 0.8);
}

.dark .audit-table thead th {
  color: #adc3e0;
  border-bottom-color: rgb(73 106 148 / 0.3);
  background: rgb(12 32 56 / 0.62);
}

.audit-table tbody td {
  border-bottom: 1px solid rgb(148 163 184 / 0.16);
  padding: 8px 9px;
  vertical-align: top;
}

.dark .audit-table tbody td {
  border-bottom-color: rgb(73 106 148 / 0.24);
}

.audit-action {
  display: inline-flex;
  border-radius: 999px;
  padding: 3px 8px;
  font-size: 10px;
  font-weight: 800;
  background: rgb(36 161 222 / 0.18);
  color: #0f172a;
}

.dark .audit-action {
  color: #dbeafe;
  background: rgb(36 161 222 / 0.28);
}

.audit-reason {
  max-width: 380px;
  overflow-wrap: anywhere;
  color: #334155;
}

.dark .audit-reason {
  color: #dbeafe;
}

@media (max-width: 1080px) {
  .finance-reconcile-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 768px) {
  .finance-filter-actions > .uc-btn {
    flex: 1 1 calc(50% - 4px);
    min-width: 0;
  }

  .finance-filter-actions > .uc-btn:last-child:nth-child(odd) {
    flex-basis: 100%;
  }

  .uc-card-title {
    font-size: 14px;
  }

  .tx-card {
    padding: 10px;
  }

  .tx-body {
    font-size: 11px;
    gap: 8px;
  }

  .tx-meta {
    font-size: 11px;
  }

  .tx-tag {
    font-size: 10px;
  }

  .token-summary-head {
    font-size: 11px;
  }

  .token-summary-desc {
    font-size: 10px;
  }

  .finance-row-actions {
    justify-content: flex-start;
  }

  .finance-table {
    min-width: 560px;
    font-size: 12px;
  }

  .reports-grid,
  .audit-filters {
    grid-template-columns: 1fr;
  }

  .audit-table {
    min-width: 620px;
  }

  .finance-table thead th,
  .finance-table tbody td {
    padding: 8px 8px;
  }

  .finance-network-pill {
    font-size: 10px;
    padding: 3px 8px;
  }
}
</style>
