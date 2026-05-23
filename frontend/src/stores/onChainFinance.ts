import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'

export interface OnChainMonitor {
  monitor_id: number
  user_id: number
  address: string
  network: 'trc20' | 'bsc' | string
  remark: string
  inflow_20: number
  outflow_20: number
  profit_20: number
}

export interface OnChainTransaction {
  ledger_id: string
  tx_id: string
  monitor_id: number
  address: string
  network: string
  direction: 'in' | 'out'
  amount: number
  token_symbol: string
  tx_timestamp: number
  counterparty?: string
  counterparty_label?: string
  counterparty_tx_count?: number
  risk_level?: string
  risk_note?: string
  income_reason?: string
  expense_reason?: string
  payer?: string
  payee?: string
  created_at?: string
}

export interface TokenSummaryItem {
  token_symbol: string
  network: string
  tx_count: number
  inflow: number
  outflow: number
  profit: number
}

export interface FinanceSummary {
  total_unreconciled: number
  total_inflow: number
  total_outflow: number
  daily_flow: number
  success_rate: number
  tx_count: number
  period_start_ms?: number
  period_end_ms?: number
  by_token: TokenSummaryItem[]
}

export interface FinanceAuditLog {
  audit_id: number
  timestamp: string
  admin_userid: number
  action: string
  target_table: string
  target_id?: string
  reason?: string
  ip_address?: string
  before_state?: string
  after_state?: string
}

interface TimelineLock {
  start: string
  end: string
}

type QueryPeriod = 'day' | 'week' | 'month' | 'custom'
type ReportFormat = 'pdf' | 'excel'

const TIMELINE_LOCK_KEY = 'onchain_finance_timeline_lock_v1'

function readTimelineLock(): TimelineLock | null {
  try {
    const raw = sessionStorage.getItem(TIMELINE_LOCK_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw)
    if (!parsed?.start || !parsed?.end) return null
    return { start: String(parsed.start), end: String(parsed.end) }
  } catch {
    return null
  }
}

function saveTimelineLock(lock: TimelineLock | null) {
  if (!lock) {
    sessionStorage.removeItem(TIMELINE_LOCK_KEY)
    return
  }
  sessionStorage.setItem(TIMELINE_LOCK_KEY, JSON.stringify(lock))
}

export const useOnChainFinanceStore = defineStore('onChainFinance', () => {
  const monitors = ref<OnChainMonitor[]>([])
  const selectedMonitorIds = ref<number[]>([])
  const isLoadingMonitors = ref(false)

  const transactions = ref<OnChainTransaction[]>([])
  const isLoadingTransactions = ref(false)

  const availableTokens = ref<string[]>(['USDT'])
  const selectedTransaction = ref<OnChainTransaction | null>(null)
  const selectedAccountCode = ref<string>('')
  const activeTab = ref<'monitors' | 'reconciliation' | 'reports' | 'audit'>('monitors')
  const error = ref<string | null>(null)
  const auditLogs = ref<FinanceAuditLog[]>([])
  const isLoadingAuditLogs = ref(false)
  const isGeneratingReport = ref(false)

  const focusedMonitorId = ref<number | undefined>(undefined)
  const timelineLock = ref<TimelineLock | null>(readTimelineLock())
  const pendingSyncChain = ref(false)

  const filters = reactive({
    period: 'month' as QueryPeriod,
    start: '',
    end: '',
    token_symbol: 'ALL',
    network: 'all',
    annotated_only: false,
    exclude_dust: false,
    dust_threshold: 1,
    limit: 300,
  })

  if (timelineLock.value) {
    filters.period = 'custom'
    filters.start = timelineLock.value.start
    filters.end = timelineLock.value.end
  }

  const DEFAULT_SUMMARY: FinanceSummary = {
    total_unreconciled: 0,
    total_inflow: 0,
    total_outflow: 0,
    daily_flow: 0,
    success_rate: 0,
    tx_count: 0,
    by_token: [],
  }
  const summary = ref<FinanceSummary>({ ...DEFAULT_SUMMARY })

  const API_BASE = '/api/v1/monitors/finance'
  let csrfToken = ''

  const getToken = () => {
    const raw = sessionStorage.getItem('adm_s')
    if (raw) {
      try {
        return decodeURIComponent(atob(raw))
      } catch {
        return ''
      }
    }
    return localStorage.getItem('admin_token') || ''
  }

  const baseHeaders = () => ({
    Authorization: `Bearer ${getToken()}`,
    'Content-Type': 'application/json',
  })

  const refreshCsrf = async (force = false) => {
    if (csrfToken && !force) return csrfToken
    const res = await fetch('/api/csrf-token', { headers: { Authorization: `Bearer ${getToken()}` } })
    if (!res.ok) throw new Error(`CSRF HTTP ${res.status}`)
    const data = await res.json()
    if (!data?.token) throw new Error('CSRF token 获取失败')
    csrfToken = String(data.token)
    return csrfToken
  }

  const requestJson = async (url: string, init: RequestInit = {}, withCsrf = false): Promise<any> => {
    const headers: Record<string, string> = {
      ...baseHeaders(),
      ...(init.headers as Record<string, string> | undefined),
    }
    if (withCsrf) {
      headers['X-CSRF-Token'] = await refreshCsrf()
    }

    const doFetch = async () => fetch(url, { ...init, headers })
    let res = await doFetch()

    if (withCsrf && res.status === 403) {
      headers['X-CSRF-Token'] = await refreshCsrf(true)
      res = await doFetch()
    }

    let data: any = null
    try {
      data = await res.json()
    } catch {
      data = null
    }

    if (!res.ok) {
      const msg = data?.message || `HTTP ${res.status}`
      throw new Error(msg)
    }
    return data
  }

  const parseQuery = (extra: Record<string, string | number | boolean | undefined> = {}) => {
    const p = new URLSearchParams()

    const lock = timelineLock.value
    if (lock?.start && lock?.end) {
      p.set('start', lock.start)
      p.set('end', lock.end)
    } else if (filters.period === 'custom' && filters.start && filters.end) {
      p.set('start', filters.start)
      p.set('end', filters.end)
    } else {
      p.set('period', filters.period)
    }

    if (focusedMonitorId.value !== undefined) p.set('monitor_id', String(focusedMonitorId.value))
    if (filters.token_symbol && filters.token_symbol !== 'ALL') p.set('token_symbol', filters.token_symbol)
    if (filters.network && filters.network !== 'all') p.set('network', filters.network)
    if (filters.annotated_only) p.set('annotated_only', 'true')
    if (filters.exclude_dust) {
      p.set('exclude_dust', 'true')
      p.set('dust_threshold', String(Math.max(0, Number(filters.dust_threshold || 1))))
    }
    p.set('limit', String(filters.limit))

    if (pendingSyncChain.value) {
      p.set('sync_chain', 'true')
    }

    Object.entries(extra).forEach(([k, v]) => {
      if (v === undefined || v === null || v === '') return
      p.set(k, String(v))
    })
    return p.toString()
  }

  const setPeriod = (period: QueryPeriod) => {
    filters.period = period
    if (period !== 'custom') {
      filters.start = ''
      filters.end = ''
    }
  }

  const setFocusedMonitor = (monitorId?: number) => {
    focusedMonitorId.value = monitorId
  }

  const requestSyncFromChain = () => {
    pendingSyncChain.value = true
  }

  const lockTimeline = (start: string, end: string) => {
    const lock = { start, end }
    timelineLock.value = lock
    saveTimelineLock(lock)
    filters.period = 'custom'
    filters.start = start
    filters.end = end
    requestSyncFromChain()
  }

  const clearTimelineLock = () => {
    timelineLock.value = null
    saveTimelineLock(null)
  }

  const loadTokens = async () => {
    try {
      const data = await requestJson(`${API_BASE}/tokens`)
      const tokens = Array.isArray(data?.data) ? data.data.map((x: any) => String(x).toUpperCase()) : []
      const merged = Array.from(new Set(['USDT', ...tokens]))
      availableTokens.value = merged
      if (filters.token_symbol !== 'ALL' && !merged.includes(filters.token_symbol)) {
        filters.token_symbol = 'ALL'
      }
    } catch (e) {
      console.error('[OnChainFinance] loadTokens failed:', e)
    }
  }

  const loadMonitors = async () => {
    isLoadingMonitors.value = true
    error.value = null
    try {
      const data = await requestJson(`${API_BASE}/monitors`)
      const rows = Array.isArray(data?.data) ? data.data : []
      if (filters.network !== 'all') {
        monitors.value = rows.filter((r: any) => String(r?.network || '').toLowerCase() === filters.network)
      } else {
        monitors.value = rows
      }
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] loadMonitors failed:', e)
    } finally {
      isLoadingMonitors.value = false
    }
  }

  const addMonitor = async (userId: number, address: string, network: string) => {
    try {
      await requestJson(
        `${API_BASE}/monitors`,
        {
          method: 'POST',
          body: JSON.stringify({ user_id: userId, address, network }),
        },
        true
      )
      await loadMonitors()
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] addMonitor failed:', e)
      return false
    }
  }

  const deleteMonitor = async (monitorId: number) => {
    try {
      await requestJson(
        `${API_BASE}/monitors/${monitorId}`,
        {
          method: 'DELETE',
        },
        true
      )
      await loadMonitors()
      if (focusedMonitorId.value === monitorId) {
        focusedMonitorId.value = undefined
      }
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] deleteMonitor failed:', e)
      return false
    }
  }

  const updateMonitorRemark = async (monitorId: number, remark: string) => {
    try {
      await requestJson(
        `${API_BASE}/monitors/${monitorId}`,
        {
          method: 'PATCH',
          body: JSON.stringify({ remark }),
        },
        true
      )
      await loadMonitors()
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] updateMonitorRemark failed:', e)
      return false
    }
  }

  const loadTransactions = async (monitorId?: number, opts: { syncChain?: boolean } = {}) => {
    isLoadingTransactions.value = true
    error.value = null
    try {
      if (monitorId !== undefined) focusedMonitorId.value = monitorId
      if (opts.syncChain) pendingSyncChain.value = true
      const qs = parseQuery()
      const data = await requestJson(`${API_BASE}/transactions?${qs}`)
      transactions.value = Array.isArray(data?.data) ? data.data : []
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] loadTransactions failed:', e)
    } finally {
      pendingSyncChain.value = false
      isLoadingTransactions.value = false
    }
  }

  const loadSummary = async (opts: { syncChain?: boolean } = {}) => {
    try {
      if (opts.syncChain) pendingSyncChain.value = true
      const qs = parseQuery()
      const data = await requestJson(`${API_BASE}/summary?${qs}`)
      summary.value = { ...DEFAULT_SUMMARY, ...(data?.data || {}) }
    } catch (e) {
      console.error('[OnChainFinance] loadSummary failed:', e)
    } finally {
      pendingSyncChain.value = false
    }
  }

  const reconcileTransaction = async (ledgerId: string, accountCode: string, amount: number) => {
    try {
      await requestJson(
        `${API_BASE}/reconcile`,
        {
          method: 'POST',
          body: JSON.stringify({ ledger_id: ledgerId, account_code: accountCode, amount }),
        },
        true
      )
      await loadTransactions()
      await loadSummary()
      selectedTransaction.value = null
      selectedAccountCode.value = ''
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] reconcileTransaction failed:', e)
      return false
    }
  }

  const upsertCounterpartyLabel = async (counterparty: string, label: string, network = '') => {
    try {
      await requestJson(
        `${API_BASE}/counterparty_labels`,
        {
          method: 'PUT',
          body: JSON.stringify({ counterparty, label, network }),
        },
        true
      )
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] upsertCounterpartyLabel failed:', e)
      return false
    }
  }

  const updateTransactionMeta = async (
    txId: string,
    payload: {
      income_reason?: string
      expense_reason?: string
      payer?: string
      payee?: string
      counterparty?: string
      counterparty_label?: string
      network?: string
    }
  ) => {
    try {
      await requestJson(
        `${API_BASE}/transactions/${encodeURIComponent(txId)}/meta`,
        {
          method: 'PATCH',
          body: JSON.stringify(payload),
        },
        true
      )
      await loadTransactions()
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] updateTransactionMeta failed:', e)
      return false
    }
  }

  const batchUpdateTransactionMeta = async (
    items: Array<{ tx_id: string; income_reason?: string; expense_reason?: string; payer?: string; payee?: string }>
  ) => {
    try {
      const normalized = items
        .map((it) => ({
          tx_id: String(it.tx_id || '').trim(),
          income_reason: (it.income_reason || '').trim(),
          expense_reason: (it.expense_reason || '').trim(),
          payer: (it.payer || '').trim(),
          payee: (it.payee || '').trim(),
        }))
        .filter((it) => it.tx_id)
      if (normalized.length === 0) return 0

      const data = await requestJson(
        `${API_BASE}/transactions/meta/batch`,
        {
          method: 'PUT',
          body: JSON.stringify({ transactions: normalized }),
        },
        true
      )
      await loadTransactions()
      return Number(data?.updated || 0)
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] batchUpdateTransactionMeta failed:', e)
      return 0
    }
  }

  const loadAuditLogs = async (opts: { days?: number; action?: string; target_table?: string } = {}) => {
    isLoadingAuditLogs.value = true
    error.value = null
    try {
      const p = new URLSearchParams()
      p.set('days', String(Math.min(Math.max(Number(opts.days || 30), 1), 365)))
      if (opts.action) p.set('action', String(opts.action))
      if (opts.target_table) p.set('target_table', String(opts.target_table))
      const data = await requestJson(`${API_BASE}/audit_logs?${p.toString()}`)
      auditLogs.value = Array.isArray(data?.data) ? data.data : []
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] loadAuditLogs failed:', e)
    } finally {
      isLoadingAuditLogs.value = false
    }
  }

  const generateReport = async (month: string, format: ReportFormat = 'pdf') => {
    isGeneratingReport.value = true
    error.value = null
    try {
      const qs = parseQuery({ month, format })
      const res = await fetch(`${API_BASE}/reports/generate?${qs}`, { headers: baseHeaders() })
      if (!res.ok) {
        let msg = `HTTP ${res.status}`
        try {
          const j = await res.json()
          msg = j?.message || msg
        } catch {
          // noop
        }
        throw new Error(msg)
      }
      const blob = await res.blob()
      const ext = format === 'excel' ? 'xlsx' : 'pdf'
      const filename = `链上财务报表_${month}.${ext}`
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] generateReport failed:', e)
      return false
    } finally {
      isGeneratingReport.value = false
    }
  }

  const exportTransactionsCsv = (opts: { filenamePrefix?: string; includeTimeline?: TimelineLock | null } = {}) => {
    try {
      const includeTimeline = !!opts.includeTimeline?.start && !!opts.includeTimeline?.end
      const headers = [
        ...(includeTimeline ? ['timeline_start', 'timeline_end'] : []),
        'tx_id',
        'ledger_id',
        'monitor_id',
        'address',
        'network',
        'direction',
        'amount',
        'token_symbol',
        'tx_timestamp',
        'counterparty',
        'counterparty_label',
        'income_reason',
        'expense_reason',
        'payer',
        'payee',
      ]

      const esc = (v: any) => {
        const raw = String(v ?? '')
        return `"${raw.replace(/"/g, '""')}"`
      }
      const rows = transactions.value.map((tx) => [
        ...(includeTimeline ? [opts.includeTimeline?.start || '', opts.includeTimeline?.end || ''] : []),
        tx.tx_id,
        tx.ledger_id,
        tx.monitor_id,
        tx.address,
        tx.network,
        tx.direction,
        tx.amount,
        tx.token_symbol,
        tx.tx_timestamp,
        tx.counterparty || '',
        tx.counterparty_label || '',
        tx.income_reason || '',
        tx.expense_reason || '',
        tx.payer || '',
        tx.payee || '',
      ])
      const csv = [headers.join(','), ...rows.map((r) => r.map(esc).join(','))].join('\n')
      const blob = new Blob([`\ufeff${csv}`], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)
      const d = new Date()
      const stamp = `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}_${String(d.getHours()).padStart(2, '0')}${String(d.getMinutes()).padStart(2, '0')}`
      const filenamePrefix = (opts.filenamePrefix || '链上流水').replace(/[\\/:*?"<>|]/g, '_')
      const a = document.createElement('a')
      a.href = url
      a.download = `${filenamePrefix}_${stamp}.csv`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      return true
    } catch (e: any) {
      error.value = e?.message || String(e)
      console.error('[OnChainFinance] exportTransactionsCsv failed:', e)
      return false
    }
  }

  const refreshAll = async () => {
    await Promise.all([loadTokens(), loadMonitors(), loadTransactions(), loadSummary()])
  }

  const toggleMonitorSelection = (monitorId: number) => {
    const idx = selectedMonitorIds.value.indexOf(monitorId)
    if (idx >= 0) selectedMonitorIds.value.splice(idx, 1)
    else selectedMonitorIds.value.push(monitorId)
  }

  const selectAllMonitors = () => {
    if (selectedMonitorIds.value.length === monitors.value.length) {
      selectedMonitorIds.value = []
    } else {
      selectedMonitorIds.value = monitors.value.map((m) => m.monitor_id)
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
    monitors,
    selectedMonitorIds,
    isLoadingMonitors,
    transactions,
    isLoadingTransactions,
    availableTokens,
    selectedTransaction,
    selectedAccountCode,
    activeTab,
    error,
    auditLogs,
    isLoadingAuditLogs,
    isGeneratingReport,
    focusedMonitorId,
    timelineLock,
    filters,
    summary,
    setPeriod,
    setFocusedMonitor,
    requestSyncFromChain,
    lockTimeline,
    clearTimelineLock,
    loadTokens,
    loadMonitors,
    addMonitor,
    deleteMonitor,
    updateMonitorRemark,
    loadTransactions,
    loadSummary,
    reconcileTransaction,
    upsertCounterpartyLabel,
    updateTransactionMeta,
    batchUpdateTransactionMeta,
    loadAuditLogs,
    generateReport,
    exportTransactionsCsv,
    refreshAll,
    toggleMonitorSelection,
    selectAllMonitors,
    selectTransaction,
    clearSelection,
  }
})
