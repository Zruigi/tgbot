<template>
  <div class="page-body">
    <!-- Top bar -->
    <div class="topbar-row">
      <h2>链上监控</h2>
      <div class="actions">
        <select v-model="period" class="input input-sm" style="width:110px" @change="loadAll">
          <option value="day">今日</option>
          <option value="week">本周</option>
          <option value="month">本月</option>
          <option value="year">本年</option>
        </select>
        <button class="btn btn-ghost btn-sm" @click="exportXlsx" :disabled="exporting">
          {{ exporting ? '导出中…' : '⬇ Excel' }}
        </button>
        <button class="btn btn-primary btn-sm" @click="openAdd">+ 新增地址</button>
      </div>
    </div>

    <!-- PnL Summary cards -->
    <div class="pnl-cards">
      <div class="pnl-card">
        <div class="pnl-label">总收入</div>
        <div class="pnl-value pnl-in">+{{ fmt(pnl.inflow) }} <span class="unit">USDT</span></div>
      </div>
      <div class="pnl-card">
        <div class="pnl-label">总支出</div>
        <div class="pnl-value pnl-out">-{{ fmt(pnl.outflow) }} <span class="unit">USDT</span></div>
      </div>
      <div class="pnl-card">
        <div class="pnl-label">净利润</div>
        <div class="pnl-value" :class="pnl.profit >= 0 ? 'pnl-pos' : 'pnl-neg'">
          {{ pnl.profit >= 0 ? '+' : '' }}{{ fmt(pnl.profit) }} <span class="unit">USDT</span>
        </div>
      </div>
      <div class="pnl-card">
        <div class="pnl-label">监听地址</div>
        <div class="pnl-value" style="color:var(--text-primary)">{{ store.monitors.length }}</div>
      </div>
    </div>

    <!-- Monitor address chips -->
    <div class="addr-chips">
      <button class="chip" :class="{ active: selectedMonitor === null }" @click="selectMonitor(null)">全部</button>
      <button
        v-for="m in store.monitors" :key="m.monitor_id"
        class="chip" :class="{ active: selectedMonitor === m.monitor_id }"
        @click="selectMonitor(m.monitor_id)"
      >
        <span class="badge" :class="m.network === 'trc20' ? 'badge-green' : 'badge-blue'" style="margin-right:5px">
          {{ m.network.toUpperCase() }}
        </span>
        {{ m.remark || shortAddr(m.address) }}
        <span class="chip-del" @click.stop="store.deleteMonitor(m.monitor_id)">×</span>
      </button>
    </div>

    <!-- Direction filter -->
    <div class="filter-row">
      <div class="btn-group">
        <button class="btn btn-sm" :class="direction===''?'btn-primary':'btn-ghost'" @click="setDir('')">全部</button>
        <button class="btn btn-sm" :class="direction==='in'?'btn-primary':'btn-ghost'" @click="setDir('in')">收入</button>
        <button class="btn btn-sm" :class="direction==='out'?'btn-primary':'btn-ghost'" @click="setDir('out')">支出</button>
      </div>
      <span class="tx-count">共 {{ txTotal }} 笔</span>
    </div>

    <!-- Transaction table -->
    <div class="card table-card">
      <div v-if="store.loading" class="empty-state">加载中…</div>
      <div v-else-if="txs.length === 0" class="empty-state">暂无流水记录</div>
      <table v-else class="tx-table">
        <thead>
          <tr>
            <th>时间</th><th>方向</th><th>金额</th><th>代币</th>
            <th>对方地址</th><th>备注/标注</th><th></th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="tx in txs" :key="tx.tx_id"
            :class="{ 'row-energy': tx.is_energy_fee }"
          >
            <td class="mono" style="white-space:nowrap">{{ fmtTime(tx.tx_timestamp) }}</td>
            <td>
              <span class="badge" :class="tx.direction==='in'?'badge-green':'badge-red'">
                {{ tx.direction === 'in' ? '收入' : '支出' }}
              </span>
            </td>
            <td :class="tx.direction==='in'?'amt-in':'amt-out'">
              {{ tx.direction==='in'?'+':'-' }}{{ fmt(tx.amount) }}
            </td>
            <td><span class="badge badge-gray">{{ tx.token_symbol }}</span></td>
            <td class="mono cp-cell" :title="tx.counterparty || ''">
              <span v-if="tx.counterparty_name" class="cp-name">{{ tx.counterparty_name }}</span>
              <span v-else>{{ shortAddr(tx.counterparty || '') }}</span>
            </td>
            <td class="meta-cell">
              <span v-if="tx.income_reason || tx.expense_reason" class="reason-tag">
                {{ tx.income_reason || tx.expense_reason }}
              </span>
              <span v-else class="text-muted">—</span>
            </td>
            <td>
              <button class="btn btn-ghost btn-sm" @click="openMeta(tx)">编辑</button>
            </td>
          </tr>
        </tbody>
      </table>
      <!-- Pagination -->
      <div v-if="txPages > 1" class="pagination">
        <button class="btn btn-ghost btn-sm" :disabled="txPage===1" @click="gotoPage(txPage-1)">‹</button>
        <span class="page-info">{{ txPage }} / {{ txPages }}</span>
        <button class="btn btn-ghost btn-sm" :disabled="txPage===txPages" @click="gotoPage(txPage+1)">›</button>
      </div>
    </div>

    <!-- Add monitor modal -->
    <div v-if="showAdd" class="modal-overlay" @click.self="showAdd=false">
      <div class="modal-box">
        <h3>新增监听地址</h3>
        <div class="field">
          <label>地址</label>
          <input v-model="form.address" class="input" placeholder="TRX / BEP20 地址" />
        </div>
        <div class="field">
          <label>网络</label>
          <select v-model="form.network" class="input">
            <option value="trc20">TRC20</option>
            <option value="bsc">BSC</option>
          </select>
        </div>
        <div class="field">
          <label>备注</label>
          <input v-model="form.remark" class="input" placeholder="可选备注" />
        </div>
        <div class="modal-actions">
          <button class="btn btn-ghost" @click="showAdd=false">取消</button>
          <button class="btn btn-primary" @click="submitAdd">确认</button>
        </div>
      </div>
    </div>

    <!-- Edit tx meta modal -->
    <div v-if="editTx" class="modal-overlay" @click.self="editTx=null">
      <div class="modal-box">
        <h3>流水标注</h3>
        <div class="tx-meta-summary">
          <span class="badge" :class="editTx.direction==='in'?'badge-green':'badge-red'">
            {{ editTx.direction==='in'?'收入':'支出' }}
          </span>
          <span style="margin-left:8px;font-weight:600">{{ fmt(editTx.amount) }} {{ editTx.token_symbol }}</span>
          <span class="mono" style="margin-left:12px;color:var(--text-secondary);font-size:11px">
            {{ fmtTime(editTx.tx_timestamp) }}
          </span>
        </div>
        <div class="field" style="margin-top:16px">
          <label>{{ editTx.direction==='in'?'收入原因':'支出原因' }}</label>
          <input v-if="editTx.direction==='in'" v-model="metaForm.income_reason" class="input" placeholder="如：客户入金" />
          <input v-else v-model="metaForm.expense_reason" class="input" placeholder="如：手续费" />
        </div>
        <div class="field">
          <label>打款人</label>
          <input v-model="metaForm.payer" class="input" placeholder="可选" />
        </div>
        <div class="field">
          <label>收款人</label>
          <input v-model="metaForm.payee" class="input" placeholder="可选" />
        </div>
        <div class="modal-actions">
          <button class="btn btn-ghost" @click="editTx=null">取消</button>
          <button class="btn btn-primary" @click="saveMeta">保存</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { useMonitorStore, type Transaction } from '@/stores/monitor'
import { monitorApi } from '@/api'

const store = useMonitorStore()

const period          = ref('month')
const direction       = ref('')
const selectedMonitor = ref<number | null>(null)
const txs             = ref<Transaction[]>([])
const txTotal         = ref(0)
const txPage          = ref(1)
const txPages         = ref(1)
const pnl             = reactive({ inflow: 0, outflow: 0, profit: 0 })
const exporting       = ref(false)
const showAdd         = ref(false)
const form            = reactive({ address: '', network: 'trc20', remark: '' })
const editTx          = ref<Transaction | null>(null)
const metaForm        = reactive({ income_reason: '', expense_reason: '', payer: '', payee: '' })

onMounted(() => loadAll())

async function loadAll() {
  txPage.value = 1
  await Promise.all([store.fetchMonitors(), loadTxs(), loadPnl()])
}

async function loadTxs() {
  const params: Record<string, any> = { period: period.value, page: txPage.value, page_size: 50 }
  if (selectedMonitor.value !== null) params.monitor_id = selectedMonitor.value
  if (direction.value)                params.direction  = direction.value
  const data = await store.fetchTransactions(params)
  txs.value    = data.txs
  txTotal.value = data.total
  txPages.value = data.pages
}

async function loadPnl() {
  const data = await store.fetchPnl(period.value)
  pnl.inflow  = data.inflow  ?? 0
  pnl.outflow = data.outflow ?? 0
  pnl.profit  = data.profit  ?? 0
}

function selectMonitor(id: number | null) {
  selectedMonitor.value = id; txPage.value = 1; loadTxs()
}
function setDir(d: string) {
  direction.value = d; txPage.value = 1; loadTxs()
}
async function gotoPage(p: number) {
  txPage.value = p
  await loadTxs()
  window.scrollTo({ top: 0, behavior: 'smooth' })
}

function openAdd() { showAdd.value = true }
async function submitAdd() {
  if (!form.address.trim()) return
  await monitorApi.create({ ...form, user_id: 0 })
  showAdd.value = false
  form.address = ''; form.remark = ''
  await store.fetchMonitors()
}

function openMeta(tx: Transaction) {
  editTx.value = tx
  metaForm.income_reason  = tx.income_reason  || ''
  metaForm.expense_reason = tx.expense_reason || ''
  metaForm.payer = tx.payer || ''
  metaForm.payee = tx.payee || ''
}
async function saveMeta() {
  if (!editTx.value) return
  await monitorApi.updateTxMeta({
    tx_id: editTx.value.tx_id,
    income_reason:  metaForm.income_reason  || null,
    expense_reason: metaForm.expense_reason || null,
    payer: metaForm.payer || null,
    payee: metaForm.payee || null,
  })
  const t = txs.value.find(t => t.tx_id === editTx.value!.tx_id)
  if (t) Object.assign(t, { income_reason: metaForm.income_reason || null,
    expense_reason: metaForm.expense_reason || null,
    payer: metaForm.payer || null, payee: metaForm.payee || null })
  editTx.value = null
}

async function exportXlsx() {
  exporting.value = true
  try {
    const res = await monitorApi.exportExcel({ period: period.value })
    const url = URL.createObjectURL(new Blob([res.data]))
    const a   = document.createElement('a')
    a.href = url; a.download = `monitor_${period.value}.xlsx`; a.click()
    URL.revokeObjectURL(url)
  } finally { exporting.value = false }
}

function fmt(n: number) { return (n ?? 0).toLocaleString('zh-CN', { maximumFractionDigits: 2 }) }
function shortAddr(s: string) {
  if (!s || s.length < 12) return s
  return s.slice(0, 6) + '…' + s.slice(-4)
}
function fmtTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN', { hour12: false })
}
</script>

<style scoped>
.topbar-row { display:flex; align-items:center; justify-content:space-between; margin-bottom:20px; }
.topbar-row h2 { font-size:18px; font-weight:700; }
.actions { display:flex; align-items:center; gap:8px; }
.input-sm { padding:5px 10px; font-size:12px; }

.pnl-cards {
  display:grid; grid-template-columns:repeat(4,1fr); gap:12px; margin-bottom:16px;
}
@media (max-width:900px) { .pnl-cards { grid-template-columns:repeat(2,1fr); } }
.pnl-card {
  background:var(--bg-surface); border:1px solid var(--border);
  border-radius:var(--radius-md); padding:14px 18px;
}
.pnl-label { font-size:11px; font-weight:600; color:var(--text-secondary);
  text-transform:uppercase; margin-bottom:6px; }
.pnl-value { font-size:22px; font-weight:700; font-family:var(--font-mono); }
.unit { font-size:12px; font-weight:400; color:var(--text-secondary); }
.pnl-in  { color:var(--success); }
.pnl-out { color:var(--danger); }
.pnl-pos { color:var(--success); }
.pnl-neg { color:var(--danger); }

.addr-chips { display:flex; flex-wrap:wrap; gap:6px; margin-bottom:14px; }
.chip {
  display:inline-flex; align-items:center; gap:4px;
  padding:4px 10px; border-radius:99px;
  background:var(--bg-elevated); border:1px solid var(--border);
  font-size:12px; color:var(--text-secondary); cursor:pointer; transition:all .15s;
}
.chip:hover, .chip.active {
  background:var(--bg-hover); color:var(--text-primary); border-color:var(--accent);
}
.chip-del { margin-left:4px; font-size:14px; line-height:1; color:var(--text-muted); }
.chip-del:hover { color:var(--danger); }

.filter-row { display:flex; align-items:center; gap:12px; margin-bottom:12px; }
.btn-group { display:flex; gap:4px; }
.tx-count { font-size:12px; color:var(--text-secondary); margin-left:auto; }

.table-card { padding:0; overflow:hidden; }
.tx-table { width:100%; border-collapse:collapse; font-size:13px; }
.tx-table th {
  padding:10px 12px; text-align:left; font-size:11px; font-weight:600;
  color:var(--text-secondary); text-transform:uppercase;
  background:var(--bg-elevated); border-bottom:1px solid var(--border);
}
.tx-table td { padding:9px 12px; border-bottom:1px solid var(--border-muted); vertical-align:middle; }
.tx-table tr:last-child td { border-bottom:none; }
.tx-table tr:hover td { background:var(--bg-elevated); }
.row-energy td { opacity:.55; }
.amt-in  { color:var(--success); font-weight:600; font-family:var(--font-mono); }
.amt-out { color:var(--danger);  font-weight:600; font-family:var(--font-mono); }
.cp-cell { max-width:160px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.cp-name { color:var(--accent); }
.meta-cell { max-width:140px; }
.reason-tag {
  display:inline-block; padding:2px 7px; border-radius:4px;
  background:rgba(88,166,255,.12); color:var(--accent); font-size:11px;
}
.text-muted { color:var(--text-muted); }

.empty-state { padding:40px; text-align:center; color:var(--text-muted); font-size:13px; }

.pagination {
  display:flex; align-items:center; justify-content:center; gap:12px;
  padding:12px; border-top:1px solid var(--border-muted);
}
.page-info { font-size:12px; color:var(--text-secondary); }

.field { margin-bottom:12px; }
.field label { display:block; font-size:11px; font-weight:600;
  color:var(--text-secondary); margin-bottom:5px; text-transform:uppercase; }
.modal-overlay {
  position:fixed; inset:0; background:rgba(0,0,0,.6);
  display:flex; align-items:center; justify-content:center; z-index:100;
}
.modal-box {
  background:var(--bg-surface); border:1px solid var(--border);
  border-radius:var(--radius-lg); padding:28px; width:420px; max-width:95vw;
}
.modal-box h3 { font-size:16px; font-weight:600; margin-bottom:20px; }
.modal-actions { display:flex; justify-content:flex-end; gap:8px; margin-top:20px; }
.tx-meta-summary { display:flex; align-items:center; flex-wrap:wrap; gap:4px; }
</style>
