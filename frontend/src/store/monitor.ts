import { defineStore } from 'pinia'
import { ref } from 'vue'
import { monitorApi } from '@/api'

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
    const res = await monitorApi.list()
    monitors.value = res.data
  }

  async function fetchTransactions(params: object) {
    loading.value = true
    try {
      const res = await monitorApi.getTransactions(params)
      transactions.value = res.data.txs
      return res.data
    } finally {
      loading.value = false
    }
  }

  async function fetchPnl(period: string) {
    const res = await monitorApi.getPnlSummary(period)
    pnlSummary.value = res.data.data
    return res.data.data
  }

  async function deleteMonitor(id: number) {
    await monitorApi.delete(id)
    monitors.value = monitors.value.filter(m => m.monitor_id !== id)
  }

  return { monitors, transactions, pnlSummary, loading, fetchMonitors, fetchTransactions, fetchPnl, deleteMonitor }
})
