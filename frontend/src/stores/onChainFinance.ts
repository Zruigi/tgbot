import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'

export interface OnChainMonitor {
  monitor_id: number
  user_id: number
  address: string
  network: 'trc20' | 'bsc'
  remark: string
  inflow_20: number
  outflow_20: number
  profit_20: number
}

export interface OnChainTransaction {
  ledger_id: string
  monitor_id: number
  tx_hash: string
  direction: 'in' | 'out'
  amount: number
  token_symbol: string
  base_fiat_value: number
  created_at: string
  reconciled_status: 'UNRECONCILED' | 'RECONCILED'
}

export interface FinanceSummary {
  total_unreconciled: number
  total_inflow: number
  total_outflow: number
  daily_flow: number
  success_rate: number
}

export const useOnChainFinanceStore = defineStore('onChainFinance', () => {
  // 监听相关状态
  const monitors = ref<OnChainMonitor[]>([])
  const selectedMonitorIds = ref<number[]>([])
  const isLoadingMonitors = ref(false)
  
  // 交易相关状态
  const transactions = ref<OnChainTransaction[]>([])
  const isLoadingTransactions = ref(false)
  
  // 财务统计状态
  const DEFAULT_SUMMARY: FinanceSummary = {
    total_unreconciled: 0,
    total_inflow: 0,
    total_outflow: 0,
    daily_flow: 0,
    success_rate: 0
  }

  const summary = ref<FinanceSummary>({ ...DEFAULT_SUMMARY })
  
  // 对账状态
  const selectedTransaction = ref<OnChainTransaction | null>(null)
  const selectedAccountCode = ref<string>('')
  
  // UI 状态
  const activeTab = ref<'monitors' | 'reconciliation' | 'reports' | 'audit'>('monitors')
  const error = ref<string | null>(null)
  
  const API_BASE = '/api/v1/monitors/finance'
  const getToken = () => {
    const raw = sessionStorage.getItem('adm_s')
    if (raw) {
      try {
        return decodeURIComponent(atob(raw))
      } catch {
        return ''
      }
    }
    return ''
  }
  const authH = () => ({
    'Authorization': `Bearer ${getToken()}`,
    'Content-Type': 'application/json'
  })

  // ===== 监听管理 =====
  const loadMonitors = async () => {
    isLoadingMonitors.value = true
    error.value = null
    try {
      const res = await fetch(`${API_BASE}/monitors`, { headers: authH() })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      monitors.value = data.data || []
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] loadMonitors failed:', e)
    } finally {
      isLoadingMonitors.value = false
    }
  }

  const addMonitor = async (userId: number, address: string, network: string) => {
    try {
      const res = await fetch(`${API_BASE}/monitors`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ user_id: userId, address, network })
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      await loadMonitors()
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] addMonitor failed:', e)
      return false
    }
  }

  const deleteMonitor = async (monitorId: number) => {
    try {
      const res = await fetch(`${API_BASE}/monitors/${monitorId}`, {
        method: 'DELETE',
        headers: authH()
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      await loadMonitors()
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] deleteMonitor failed:', e)
      return false
    }
  }

  const updateMonitorRemark = async (monitorId: number, remark: string) => {
    try {
      const res = await fetch(`${API_BASE}/monitors/${monitorId}`, {
        method: 'PATCH',
        headers: authH(),
        body: JSON.stringify({ remark })
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      await loadMonitors()
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] updateMonitorRemark failed:', e)
      return false
    }
  }

  // ===== 交易查询 =====
  const loadTransactions = async (monitorId?: number) => {
    isLoadingTransactions.value = true
    error.value = null
    try {
      let url = `${API_BASE}/transactions`
      if (monitorId) url += `?monitor_id=${monitorId}`
      
      const res = await fetch(url, { headers: authH() })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      transactions.value = data.data || []
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] loadTransactions failed:', e)
    } finally {
      isLoadingTransactions.value = false
    }
  }

  // ===== 财务统计 =====
  const loadSummary = async () => {
    try {
      const res = await fetch(`${API_BASE}/summary`, { headers: authH() })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      summary.value = { ...DEFAULT_SUMMARY, ...(data.data || {}) }
    } catch (e) {
      console.error('[OnChainFinance] loadSummary failed:', e)
    }
  }

  // ===== 对账操作 =====
  const reconcileTransaction = async (ledgerId: string, accountCode: string, amount: number) => {
    try {
      const res = await fetch(`${API_BASE}/reconcile`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ ledger_id: ledgerId, account_code: accountCode, amount })
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      
      await loadTransactions()
      await loadSummary()
      selectedTransaction.value = null
      selectedAccountCode.value = ''
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[OnChainFinance] reconcileTransaction failed:', e)
      return false
    }
  }

  // ===== 工具方法 =====
  const toggleMonitorSelection = (monitorId: number) => {
    const idx = selectedMonitorIds.value.indexOf(monitorId)
    if (idx >= 0) {
      selectedMonitorIds.value.splice(idx, 1)
    } else {
      selectedMonitorIds.value.push(monitorId)
    }
  }

  const selectAllMonitors = () => {
    if (selectedMonitorIds.value.length === monitors.value.length) {
      selectedMonitorIds.value = []
    } else {
      selectedMonitorIds.value = monitors.value.map(m => m.monitor_id)
    }
  }

  const selectTransaction = (tx: OnChainTransaction) => {
    selectedTransaction.value = tx
  }

  const clearSelection = () => {
    selectedTransaction.value = null
    selectedAccountCode.value = ''
  }

  return {
    // 监听
    monitors,
    selectedMonitorIds,
    isLoadingMonitors,
    loadMonitors,
    addMonitor,
    deleteMonitor,
    updateMonitorRemark,
    toggleMonitorSelection,
    selectAllMonitors,
    
    // 交易
    transactions,
    isLoadingTransactions,
    loadTransactions,
    selectTransaction,
    
    // 财务
    summary,
    loadSummary,
    
    // 对账
    selectedTransaction,
    selectedAccountCode,
    reconcileTransaction,
    clearSelection,
    
    // UI
    activeTab,
    error
  }
})
