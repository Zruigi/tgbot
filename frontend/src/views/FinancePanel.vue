<template>
  <div class="flex-1 flex flex-col h-full bg-surface overflow-hidden">
    <!-- Top Bar -->
    <div class="flex items-center justify-between px-8 py-6 border-b border-outline-variant/10 bg-surface-container-lowest/50 backdrop-blur-sm">
      <div>
        <h2 class="text-2xl font-black flex items-center gap-3">
          <span class="material-symbols-outlined text-primary text-3xl">account_balance</span>
          企业财务管理
        </h2>
        <p class="text-sm text-slate-400 mt-1">对账工作台、财务报表、审计日志一体化管理</p>
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
                @click="activeTab = tab.id"
                :class="[
                  'px-6 py-2.5 rounded-xl font-medium text-sm transition-all duration-200',
                  activeTab === tab.id
                    ? 'bg-primary text-white shadow-lg shadow-primary-container/20'
                    : 'text-outline-variant hover:text-on-surface'
                ]">
          {{ tab.label }}
        </button>
      </div>
    </div>

    <!-- Content Area -->
    <main class="flex-1 overflow-y-auto no-scrollbar px-8 py-6">
      <!-- Reconciliation Tab -->
      <div v-if="activeTab === 'reconciliation'" class="space-y-6">
        <!-- Summary Stats -->
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-on-surface font-semibold flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">list_alt</span>
            待处理流水
          </h3>
          <span class="text-xs font-medium text-primary bg-primary-container/10 px-3 py-1 rounded-full">
            {{ financeStore.summary.total_unreconciled }} 条待对账
          </span>
        </div>

        <!-- Grid Layout -->
        <div class="grid grid-cols-1 lg:grid-cols-12 gap-6">
          <!-- Left: Transaction Cards -->
          <div class="lg:col-span-7 space-y-4">
            <div v-if="financeStore.isLoading" class="flex flex-col items-center justify-center py-20">
              <div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
            </div>
            <div v-else-if="financeStore.unreconciled.length === 0" class="flex flex-col items-center justify-center py-20 text-slate-300 italic">
              <span class="material-symbols-outlined text-6xl mb-4 opacity-20">check_circle</span>
              <p>所有流水已对账</p>
            </div>
            <TransactionCard v-for="ledger in financeStore.unreconciled"
                            :key="ledger.ledger_id"
                            :ledger="ledger"
                            @select="selectLedger(ledger)" />
          </div>

          <!-- Right: Reconciliation Panel -->
          <div class="lg:col-span-5">
            <div class="sticky top-6 space-y-6">
              <ReconciliationPanel v-if="selectedLedger"
                                  :ledger-id="selectedLedger.ledger_id"
                                  :amount="selectedLedger.amount"
                                  :token-symbol="selectedLedger.token_symbol"
                                  :categories="financeStore.accountCategories"
                                  @confirmed="handleReconciliationConfirmed"
                                  @error="handleError" />
              <div v-else class="bg-surface-container-lowest/90 backdrop-blur-md p-6 rounded-2xl shadow-xl border border-primary/5 text-center py-12">
                <span class="material-symbols-outlined text-6xl text-outline-variant/30 mb-4 block">touch_app</span>
                <p class="text-sm text-outline-variant">选择左侧流水开始对账</p>
              </div>

              <!-- Stats Bento -->
              <StatsCard :total-unreconciled="financeStore.summary.total_unreconciled"
                        :total-inflow="financeStore.summary.total_inflow"
                        :total-outflow="financeStore.summary.total_outflow" />
            </div>
          </div>
        </div>
      </div>

      <!-- Reports Tab -->
      <div v-if="activeTab === 'reports'" class="max-w-2xl">
        <div class="bg-surface-container-lowest/90 backdrop-blur-md p-8 rounded-2xl shadow-xl border border-primary/5">
          <h3 class="text-lg font-bold text-on-surface mb-8 flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">insert_chart</span>
            报表生成中心
          </h3>

          <div class="space-y-6">
            <!-- Month Selector -->
            <div class="flex flex-col gap-2">
              <label class="text-xs font-bold text-outline uppercase tracking-wider">选择月份</label>
              <input v-model="reportMonth" type="month" class="w-full bg-surface-container-low border-none rounded-xl py-3 px-4 text-sm font-medium focus:ring-2 focus:ring-primary-container/40">
            </div>

            <!-- Format Selector -->
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

            <!-- Generate Button -->
            <button @click="handleGenerateReport"
                    :disabled="financeStore.isGenerating"
                    class="w-full py-4 bg-gradient-to-r from-primary to-primary-container text-white rounded-xl font-bold text-lg shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all disabled:opacity-50 disabled:cursor-not-allowed">
              {{ financeStore.isGenerating ? '生成中...' : '生成报表' }}
            </button>
          </div>
        </div>
      </div>

      <!-- Audit Tab -->
      <div v-if="activeTab === 'audit'" class="space-y-4">
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-lg font-bold text-on-surface flex items-center gap-2">
            <span class="material-symbols-outlined text-primary">history_edu</span>
            审计日志
          </h3>
          <button @click="financeStore.loadAuditLogs()" class="px-4 py-2 bg-primary text-white rounded-xl text-sm font-bold hover:brightness-105 transition-all">
            刷新
          </button>
        </div>
        <div class="bg-surface-container-lowest/90 backdrop-blur-md rounded-2xl shadow-xl border border-primary/5 overflow-hidden">
          <AuditLogTable :logs="financeStore.auditLogs" />
        </div>
      </div>
    </main>

    <!-- Error Toast -->
    <transition name="fade">
      <div v-if="errorMessage" class="fixed bottom-6 right-6 bg-error text-on-error px-6 py-4 rounded-2xl shadow-lg flex items-center gap-3 animate-in slide-in-from-bottom">
        <span class="material-symbols-outlined">error</span>
        <span class="text-sm font-bold">{{ errorMessage }}</span>
        <button @click="errorMessage = ''" class="ml-2 opacity-70 hover:opacity-100">
          <span class="material-symbols-outlined text-lg">close</span>
        </button>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useFinanceStore } from '@/stores/finance'
import TransactionCard from '@/components/finance/TransactionCard.vue'
import ReconciliationPanel from '@/components/finance/ReconciliationPanel.vue'
import StatsCard from '@/components/finance/StatsCard.vue'
import AuditLogTable from '@/components/finance/AuditLogTable.vue'
import type { UnreconciledLedger } from '@/stores/finance'

const financeStore = useFinanceStore()

const activeTab = ref('reconciliation')
const selectedLedger = ref<UnreconciledLedger | null>(null)
const reportMonth = ref(new Date().toISOString().substring(0, 7))
const reportFormat = ref<'pdf' | 'excel'>('pdf')
const errorMessage = ref('')

const tabs = [
  { id: 'reconciliation', label: '✓ 对账工作台' },
  { id: 'reports', label: '📊 财务报表' },
  { id: 'audit', label: '📋 审计日志' }
]

const selectLedger = (ledger: UnreconciledLedger) => {
  selectedLedger.value = ledger
  financeStore.selectLedger(ledger.ledger_id, ledger.amount)
}

const handleReconciliationConfirmed = () => {
  selectedLedger.value = null
  errorMessage.value = ''
}

const handleError = (message: string) => {
  errorMessage.value = message
}

const handleRefresh = async () => {
  await Promise.all([
    financeStore.loadUnreconciled(),
    financeStore.loadSummary(),
    financeStore.loadAuditLogs()
  ])
}

const handleGenerateReport = async () => {
  const success = await financeStore.generateReport(reportMonth.value, reportFormat.value)
  if (!success) {
    errorMessage.value = financeStore.error || '报表生成失败'
  }
}

onMounted(async () => {
  await Promise.all([
    financeStore.loadUnreconciled(),
    financeStore.loadSummary(),
    financeStore.loadAuditLogs(),
    financeStore.loadAccountCategories()
  ])
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
