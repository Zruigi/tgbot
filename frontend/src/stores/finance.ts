import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'

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

export interface FinanceSummary {
  total_unreconciled: number
  total_inflow: number
  total_outflow: number
}

export interface AuditLog {
  audit_id: number
  timestamp: string
  admin_userid: number
  action: string
  target_table: string
}

export interface AccountCategory {
  id: string
  code: string
  name: string
  type: 'asset' | 'liability' | 'equity' | 'revenue' | 'expense'
}

export const useFinanceStore = defineStore('finance', () => {
  // State
  const unreconciled = ref<UnreconciledLedger[]>([])
  const summary = ref<FinanceSummary>({
    total_unreconciled: 0,
    total_inflow: 0,
    total_outflow: 0
  })
  const auditLogs = ref<AuditLog[]>([])
  const accountCategories = ref<AccountCategory[]>([])
  
  const isLoading = ref(false)
  const isGenerating = ref(false)
  const error = ref<string | null>(null)

  // Reconciliation state
  const selectedLedgerId = ref<number | null>(null)
  const selectedAccountCode = ref<string>('')
  const allocationAmount = ref<number>(0)

  // API base path
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

  // Actions
  const loadUnreconciled = async () => {
    isLoading.value = true
    error.value = null
    try {
      const res = await fetch(`${API_BASE}/unreconciled`, {
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        }
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      unreconciled.value = data.data || []
    } catch (e) {
      error.value = String(e)
      console.error('[Finance] loadUnreconciled failed:', e)
    } finally {
      isLoading.value = false
    }
  }

  const loadSummary = async () => {
    try {
      const res = await fetch(`${API_BASE}/summary`, {
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        }
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      summary.value = data.data || {}
    } catch (e) {
      console.error('[Finance] loadSummary failed:', e)
    }
  }

  const loadAuditLogs = async () => {
    try {
      const res = await fetch(`${API_BASE}/audit_logs`, {
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        }
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      auditLogs.value = data.data || []
    } catch (e) {
      console.error('[Finance] loadAuditLogs failed:', e)
    }
  }

  const loadAccountCategories = async () => {
    try {
      const res = await fetch(`${API_BASE}/account_categories`, {
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        }
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()
      accountCategories.value = data.data || []
    } catch (e) {
      console.error('[Finance] loadAccountCategories failed:', e)
    }
  }

  const confirmReconciliation = async (ledgerId: number, accountCode: string, amount: number) => {
    try {
      const res = await fetch(`${API_BASE}/reconcile`, {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          ledger_id: ledgerId,
          account_code: accountCode,
          amount
        })
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      await loadUnreconciled()
      await loadSummary()
      selectedLedgerId.value = null
      selectedAccountCode.value = ''
      allocationAmount.value = 0
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[Finance] confirmReconciliation failed:', e)
      return false
    }
  }

  const generateReport = async (month: string, format: 'pdf' | 'excel') => {
    isGenerating.value = true
    error.value = null
    try {
      const url = `${API_BASE}/reports/generate?month=${month}&format=${format}`
      const res = await fetch(url, {
        headers: {
          'Authorization': `Bearer ${getToken()}`,
          'Content-Type': 'application/json'
        }
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const blob = await res.blob()
      const downloadUrl = window.URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = downloadUrl
      a.download = `财务报表_${month}.${format === 'pdf' ? 'pdf' : 'xlsx'}`
      a.click()
      window.URL.revokeObjectURL(downloadUrl)
      return true
    } catch (e) {
      error.value = String(e)
      console.error('[Finance] generateReport failed:', e)
      return false
    } finally {
      isGenerating.value = false
    }
  }

  const selectLedger = (ledgerId: number, amount: number) => {
    selectedLedgerId.value = ledgerId
    allocationAmount.value = amount
  }

  const clearSelection = () => {
    selectedLedgerId.value = null
    selectedAccountCode.value = ''
    allocationAmount.value = 0
  }

  return {
    // State
    unreconciled,
    summary,
    auditLogs,
    accountCategories,
    isLoading,
    isGenerating,
    error,
    selectedLedgerId,
    selectedAccountCode,
    allocationAmount,
    // Actions
    loadUnreconciled,
    loadSummary,
    loadAuditLogs,
    loadAccountCategories,
    confirmReconciliation,
    generateReport,
    selectLedger,
    clearSelection
  }
})
