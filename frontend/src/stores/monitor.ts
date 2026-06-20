import { defineStore } from 'pinia'
import { ref } from 'vue'

const token = () => localStorage.getItem('admin_token') || ''
const authH = () => ({ 'Authorization': `Bearer ${token()}`, 'Content-Type': 'application/json' })

export interface Monitor {
  monitor_id: number
  user_id: number
  address: string
  network: string
  remark: string
  risk_level: string
  is_active: boolean
  inflow_20: number
  outflow_20: number
  profit_20: number
}

export interface Transaction {
  tx_id: string
  monitor_id: number
  direction: string
  amount: number
  token_symbol: string
  tx_timestamp: number
  counterparty: string | null
  counterparty_name: string
  income_reason: string | null
  expense_reason: string | null
  payer: string | null
  payee: string | null
  is_energy_fee: boolean
}

export const useMonitorStore = defineStore('monitor', () => {
  const monitors = ref<Monitor[]>([])
  const transactions = ref<Transaction[]>([])
  const pnlSummary = ref<Record<string, any>>({})
  const loading = ref(false)

  async function fetchMonitors() {
    const res = await fetch('/api/monitors', { headers: authH() })
    const d = await res.json()
    monitors.value = d.data || []
  }

  async function fetchTransactions(params: Record<string, any>) {
    loading.value = true
    try {
      const qs = new URLSearchParams(params).toString()
      const res = await fetch(`/api/monitors/transactions?${qs}`, { headers: authH() })
      const d = await res.json()
      transactions.value = d.data?.txs || []
      return d.data
    } finally {
      loading.value = false
    }
  }

  async function fetchPnl(period: string) {
    const res = await fetch(`/api/monitors/pnl_summary?period=${period}`, { headers: authH() })
    const d = await res.json()
    pnlSummary.value = d.data?.data || {}
    return pnlSummary.value
  }

  async function deleteMonitor(id: number) {
    await fetch(`/api/monitors/${id}`, { method: 'DELETE', headers: authH() })
    monitors.value = monitors.value.filter(m => m.monitor_id !== id)
  }

  return { monitors, transactions, pnlSummary, loading, fetchMonitors, fetchTransactions, fetchPnl, deleteMonitor }
})
