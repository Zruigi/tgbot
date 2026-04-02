<template>
  <div class="flex-1 flex flex-col h-full bg-surface overflow-hidden">
    <!-- Top Bar -->
    <div class="flex items-center justify-between px-8 py-6 border-b border-outline-variant/10 bg-surface-container-lowest/50 backdrop-blur-sm">
      <div>
        <h2 class="text-2xl font-black flex items-center gap-3">
          <span class="material-symbols-outlined text-primary text-3xl">account_balance_wallet</span>
          链上财务管理
        </h2>
        <p class="text-sm text-slate-400 mt-1">链上监听 → 自动记账 → 对账 → 报表一体化管理</p>
      </div>
      <div class="flex items-center space-x-3">
        <button @click="handleRefresh" class="p-3 bg-surface-container hover:bg-surface-container-high rounded-2xl transition-all">
          <span class="material-symbols-outlined text-primary">refresh</span>
        </button>
      </div>
    </div>

    <!-- Tab Navigation -->
    <div class="px-8 py-4 overflow-x-auto no-scrollbar border-b border-outline-variant/10">
      <div class="flex items-center space-x-2 min-w-max p-1 bg-surface-container-low rounded-2xl">
        <button v-for="tab in tabs" :key="tab.id"
                @click="store.activeTab = tab.id"
                :class="[
                  'px-6 py-2.5 rounded-xl font-medium text-sm transition-all duration-200',
                  store.activeTab === tab.id
                    ? 'bg-primary text-white shadow-lg shadow-primary-container/20'
                    : 'text-outline-variant hover:text-on-surface'
                ]">
          {{ tab.label }}
        </button>
      </div>
    </div>

    <!-- Content Area -->
    <main class="flex-1 overflow-y-auto no-scrollbar px-8 py-6">
      <!-- Monitors Tab -->
      <div v-if="store.activeTab === 'monitors'" class="space-y-6">
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-lg font-bold text-on-surface flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">monitoring</span>
            链上地址监听
          </h3>
          <button @click="showAddMonitorModal = true" class="px-4 py-2 bg-primary text-white rounded-xl text-sm font-bold">
            <span class="material-symbols-outlined text-sm mr-1">add</span>新增监听
          </button>
        </div>

        <!-- Stats Grid -->
        <div class="grid grid-cols-1 md:grid-cols-4 gap-4">
          <div class="bg-surface-container-lowest p-4 rounded-2xl border border-outline-variant/10">
            <span class="text-[10px] font-bold text-outline-variant uppercase">监听总数</span>
            <div class="text-2xl font-bold text-on-surface mt-1">{{ store.monitors.length }}</div>
          </div>
          <div class="bg-surface-container-lowest p-4 rounded-2xl border border-outline-variant/10">
            <span class="text-[10px] font-bold text-outline-variant uppercase">日流量</span>
            <div class="text-2xl font-bold text-on-surface mt-1">{{ store.summary.daily_flow }} ETH</div>
          </div>
          <div class="bg-surface-container-lowest p-4 rounded-2xl border border-outline-variant/10">
            <span class="text-[10px] font-bold text-outline-variant uppercase">总入金</span>
            <div class="text-2xl font-bold text-green-600 mt-1">${{ store.summary.total_inflow.toFixed(2) }}</div>
          </div>
          <div class="bg-surface-container-lowest p-4 rounded-2xl border border-outline-variant/10">
            <span class="text-[10px] font-bold text-outline-variant uppercase">总出金</span>
            <div class="text-2xl font-bold text-red-600 mt-1">${{ store.summary.total_outflow.toFixed(2) }}</div>
          </div>
        </div>

        <!-- Monitors Table -->
        <div class="bg-surface-container-lowest rounded-2xl border border-outline-variant/10 overflow-hidden">
          <div v-if="store.isLoadingMonitors" class="flex justify-center py-20">
            <div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
          </div>
          <div v-else-if="store.monitors.length === 0" class="flex flex-col items-center justify-center py-20 text-slate-300">
            <span class="material-symbols-outlined text-6xl mb-4 opacity-20">wallet_off</span>
            <p>暂无链上监听任务</p>
          </div>
          <div v-else class="overflow-x-auto">
            <table class="w-full text-left border-collapse text-sm">
              <thead>
                <tr class="bg-surface-container-low border-b border-outline-variant/10">
                  <th class="px-4 py-4 text-[10px] font-black uppercase text-slate-400">
                    <input type="checkbox" @change="store.selectAllMonitors()" class="rounded text-primary"/>
                  </th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">监听ID</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">地址</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">网络</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">近20笔</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400 text-right">操作</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-outline-variant/5">
                <tr v-for="m in store.monitors" :key="m.monitor_id" class="hover:bg-primary/5 transition-colors">
                  <td class="px-4 py-4">
                    <input type="checkbox" 
                           :checked="store.selectedMonitorIds.includes(m.monitor_id)"
                           @change="store.toggleMonitorSelection(m.monitor_id)"
                           class="rounded text-primary"/>
                  </td>
                  <td class="px-6 py-4 text-xs font-bold">#{{ m.monitor_id }}</td>
                  <td class="px-6 py-4 text-xs font-mono truncate max-w-[200px]">{{ m.address }}</td>
                  <td class="px-6 py-4">
                    <span class="px-2 py-1 bg-primary/10 text-primary text-[10px] font-black rounded-lg">
                      {{ m.network.toUpperCase() }}
                    </span>
                  </td>
                  <td class="px-6 py-4 text-xs">
                    入 {{ m.inflow_20 }} / 出 {{ m.outflow_20 }}
                    <div :class="m.profit_20 >= 0 ? 'text-green-600 font-bold' : 'text-red-600 font-bold'">
                      {{ m.profit_20 >= 0 ? '+' : '' }}{{ m.profit_20 }}
                    </div>
                  </td>
                  <td class="px-6 py-4 text-right">
                    <button @click="store.deleteMonitor(m.monitor_id)" 
                            class="px-3 py-2 bg-error/10 text-error text-[10px] font-black rounded-xl hover:bg-error/20">
                      删除
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>

      <!-- Reconciliation Tab -->
      <div v-if="store.activeTab === 'reconciliation'" class="space-y-6">
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-lg font-bold text-on-surface flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">list_alt</span>
            待对账流水
          </h3>
          <span class="text-xs font-medium text-primary bg-primary-container/10 px-3 py-1 rounded-full">
            {{ store.summary.total_unreconciled }} 条待对账
          </span>
        </div>

        <div class="grid grid-cols-1 lg:grid-cols-12 gap-6">
          <!-- Left: Transactions -->
          <div class="lg:col-span-7 space-y-4">
            <div v-if="store.isLoadingTransactions" class="flex justify-center py-20">
              <div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
            </div>
            <div v-else-if="store.transactions.length === 0" class="flex flex-col items-center justify-center py-20 text-slate-300">
              <span class="material-symbols-outlined text-6xl mb-4 opacity-20">check_circle</span>
              <p>所有流水已对账</p>
            </div>
            <div v-else>
              <div v-for="tx in store.transactions" :key="tx.ledger_id"
                   @click="store.selectTransaction(tx)"
                   class="group bg-surface-container-lowest/90 backdrop-blur-md p-5 rounded-2xl shadow-sm border border-outline-variant/10 hover:shadow-md transition-all duration-300 hover:scale-[1.01] cursor-pointer"
                   :class="store.selectedTransaction?.ledger_id === tx.ledger_id ? 'ring-2 ring-primary' : ''">
                <div class="flex justify-between items-start mb-3">
                  <div class="flex flex-col">
                    <span class="text-[10px] text-outline uppercase font-bold tracking-widest">Transaction ID</span>
                    <span class="text-sm font-mono font-semibold text-on-surface">{{ tx.ledger_id }}</span>
                  </div>
                  <div class="bg-error-container/20 text-error text-[10px] font-bold px-2 py-1 rounded-md">
                    {{ tx.reconciled_status }}
                  </div>
                </div>
                <div class="flex items-end justify-between">
                  <div>
                    <div class="text-2xl font-bold text-on-surface tracking-tight">{{ tx.amount }} {{ tx.token_symbol }}</div>
                    <div class="text-xs text-outline-variant flex items-center gap-1 mt-1">
                      <span class="material-symbols-outlined text-[14px]">schedule</span>
                      {{ new Date(tx.created_at).toLocaleString('zh-CN') }}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Right: Reconciliation Panel -->
          <div class="lg:col-span-5">
            <div class="sticky top-6 space-y-6">
              <div v-if="store.selectedTransaction" class="bg-surface-container-lowest/90 backdrop-blur-md p-6 rounded-2xl shadow-xl border border-primary/5">
                <h3 class="text-lg font-bold text-on-surface mb-6 flex items-center gap-2">
                  <span class="material-symbols-outlined text-primary">account_balance</span>
                  对账操作面板
                </h3>
                <div class="space-y-4">
                  <div class="flex flex-col gap-2">
                    <label class="text-xs font-bold text-outline uppercase tracking-wider">会计科目</label>
                    <select v-model="store.selectedAccountCode" 
                            class="w-full bg-surface-container-low border-none rounded-xl py-3 px-4 text-sm font-medium focus:ring-2 focus:ring-primary-container/40">
                      <option value="">-- 选择科目 --</option>
                      <option value="1001">1001 - 现金</option>
                      <option value="1002">1002 - 银行存款</option>
                      <option value="1010">1010 - 应收账款</option>
                      <option value="1020">1020 - 库存资产</option>
                    </select>
                  </div>
                  <div class="flex flex-col gap-2">
                    <label class="text-xs font-bold text-outline uppercase tracking-wider">对账金额</label>
                    <div class="relative">
                      <input class="w-full bg-surface-container-low border-none rounded-xl py-4 px-4 text-xl font-bold text-on-surface focus:ring-2 focus:ring-primary-container/40" 
                             type="text" 
                             :value="store.selectedTransaction?.amount"
                             readonly/>
                      <span class="absolute right-4 top-1/2 -translate-y-1/2 font-bold text-primary">
                        {{ store.selectedTransaction?.token_symbol }}
                      </span>
                    </div>
                  </div>
                  <button @click="handleReconcile"
                          :disabled="!store.selectedAccountCode || isReconciling"
                          class="w-full py-4 bg-gradient-to-r from-primary to-primary-container text-white rounded-xl font-bold text-lg shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all disabled:opacity-50 disabled:cursor-not-allowed">
                    {{ isReconciling ? '处理中...' : '确认对账' }}
                  </button>
                </div>
              </div>
              <div v-else class="bg-surface-container-lowest/90 backdrop-blur-md p-6 rounded-2xl shadow-xl border border-primary/5 text-center py-12">
                <span class="material-symbols-outlined text-6xl text-outline-variant/30 mb-4 block">touch_app</span>
                <p class="text-sm text-outline-variant">选择左侧流水开始对账</p>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Reports Tab -->
      <div v-if="store.activeTab === 'reports'" class="max-w-2xl">
        <div class="bg-surface-container-lowest/90 backdrop-blur-md p-8 rounded-2xl shadow-xl border border-primary/5">
          <h3 class="text-lg font-bold text-on-surface mb-8 flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">insert_chart</span>
            财务报表生成
          </h3>
          <div class="space-y-6">
            <div class="flex flex-col gap-2">
              <label class="text-xs font-bold text-outline uppercase tracking-wider">选择月份</label>
              <input v-model="reportMonth" type="month" class="w-full bg-surface-container-low border-none rounded-xl py-3 px-4 text-sm font-medium focus:ring-2 focus:ring-primary-container/40">
            </div>
            <div class="flex flex-col gap-2">
              <label class="text-xs font-bold text-outline uppercase tracking-wider">报表格式</label>
              <div class="flex gap-4">
                <label class="flex items-center gap-2 cursor-pointer">
                  <input v-model="reportFormat" type="radio" value="pdf" class="w-4 h-4 text-primary">
                  <span class="text-sm">📄 PDF</span>
                </label>
                <label class="flex items-center gap-2 cursor-pointer">
                  <input v-model="reportFormat" type="radio" value="excel" class="w-4 h-4 text-primary">
                  <span class="text-sm">📊 Excel</span>
                </label>
              </div>
            </div>
            <button @click="handleGenerateReport"
                    :disabled="isGeneratingReport"
                    class="w-full py-4 bg-gradient-to-r from-primary to-primary-container text-white rounded-xl font-bold text-lg shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all disabled:opacity-50 disabled:cursor-not-allowed">
              {{ isGeneratingReport ? '生成中...' : '生成报表' }}
            </button>
          </div>
        </div>
      </div>

      <!-- Audit Tab -->
      <div v-if="store.activeTab === 'audit'" class="space-y-4">
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-lg font-bold text-on-surface flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">history_edu</span>
            审计日志
          </h3>
          <button @click="handleRefresh" class="px-4 py-2 bg-primary text-white rounded-xl text-sm font-bold">
            刷新
          </button>
        </div>
        <div class="bg-surface-container-lowest/90 backdrop-blur-md rounded-2xl shadow-xl border border-primary/5 overflow-hidden">
          <div class="overflow-x-auto">
            <table class="w-full text-left border-collapse text-sm">
              <thead>
                <tr class="bg-surface-container-low border-b border-outline-variant/10">
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">时间</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">操作人</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">操作类型</th>
                  <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">目标表</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-outline-variant/5">
                <tr class="hover:bg-primary/5 transition-colors">
                  <td class="px-6 py-4 text-xs text-on-surface">暂无数据</td>
                  <td class="px-6 py-4"></td>
                  <td class="px-6 py-4"></td>
                  <td class="px-6 py-4"></td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </main>

    <!-- Add Monitor Modal -->
    <transition name="fade">
      <div v-if="showAddMonitorModal" class="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/50 backdrop-blur-sm">
        <div class="bg-surface-container-lowest w-full max-w-md rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95">
          <div class="px-8 py-6 border-b flex items-center justify-between">
            <h3 class="text-lg font-black">新增链上监听</h3>
            <button @click="showAddMonitorModal = false" class="w-10 h-10 flex items-center justify-center hover:bg-surface-container-high rounded-full">
              <span class="material-symbols-outlined">close</span>
            </button>
          </div>
          <div class="p-6 space-y-4">
            <input v-model="newMonitor.user_id" type="number" placeholder="用户 ID" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm"/>
            <input v-model="newMonitor.address" type="text" placeholder="USDT 地址" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm"/>
            <select v-model="newMonitor.network" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm">
              <option value="trc20">TRC20</option>
              <option value="bsc">BSC</option>
            </select>
          </div>
          <div class="px-8 py-5 bg-surface-container-low flex justify-end space-x-3 border-t">
            <button @click="showAddMonitorModal = false" class="px-5 py-2.5 text-sm font-bold text-slate-500">取消</button>
            <button @click="handleAddMonitor" class="px-6 py-2.5 bg-primary text-on-primary text-sm font-black rounded-xl">添加</button>
          </div>
        </div>
      </div>
    </transition>

    <!-- Error Toast -->
    <transition name="fade">
      <div v-if="store.error" class="fixed bottom-6 right-6 bg-error text-on-error px-6 py-4 rounded-2xl shadow-lg flex items-center gap-3 animate-in slide-in-from-bottom">
        <span class="material-symbols-outlined">error</span>
        <span class="text-sm font-bold">{{ store.error }}</span>
        <button @click="store.error = null" class="ml-2 opacity-70 hover:opacity-100">
          <span class="material-symbols-outlined text-lg">close</span>
        </button>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useOnChainFinanceStore } from '@/stores/onChainFinance'

const store = useOnChainFinanceStore()

const tabs = [
  { id: 'monitors', label: '📍 链上监听' },
  { id: 'reconciliation', label: '✓ 对账工作台' },
  { id: 'reports', label: '📊 财务报表' },
  { id: 'audit', label: '📋 审计日志' }
]

const showAddMonitorModal = ref(false)
const newMonitor = ref({ user_id: '', address: '', network: 'trc20' })
const reportMonth = ref(new Date().toISOString().substring(0, 7))
const reportFormat = ref<'pdf' | 'excel'>('pdf')
const isReconciling = ref(false)
const isGeneratingReport = ref(false)

const handleRefresh = async () => {
  await Promise.all([
    store.loadMonitors(),
    store.loadTransactions(),
    store.loadSummary()
  ])
}

const handleAddMonitor = async () => {
  if (!newMonitor.value.user_id || !newMonitor.value.address) {
    store.error = '请填写所有必填项'
    return
  }
  
  const success = await store.addMonitor(
    parseInt(newMonitor.value.user_id),
    newMonitor.value.address,
    newMonitor.value.network
  )
  
  if (success) {
    showAddMonitorModal.value = false
    newMonitor.value = { user_id: '', address: '', network: 'trc20' }
  }
}

const handleReconcile = async () => {
  if (!store.selectedTransaction || !store.selectedAccountCode) return
  
  isReconciling.value = true
  const success = await store.reconcileTransaction(
    store.selectedTransaction.ledger_id,
    store.selectedAccountCode,
    store.selectedTransaction.amount
  )
  isReconciling.value = false
  
  if (!success) {
    store.error = '对账失败，请重试'
  }
}

const handleGenerateReport = async () => {
  isGeneratingReport.value = true
  // TODO: 实现报表生成逻辑
  isGeneratingReport.value = false
}

onMounted(async () => {
  await handleRefresh()
})
</script>

<style scoped>
.no-scrollbar {
  -ms-overflow-style: none;
  scrollbar-width: none;
}

.no-scrollbar::-webkit-scrollbar {
  display: none;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.3s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.slide-in-from-bottom-enter-active {
  animation: slideInFromBottom 0.3s ease;
}

@keyframes slideInFromBottom {
  from {
    transform: translateY(100%);
    opacity: 0;
  }
  to {
    transform: translateY(0);
    opacity: 1;
  }
}
</style>
