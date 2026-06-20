<template>
  <div class="flex-1 flex flex-col p-10 animate-in slide-in-from-right duration-500 overflow-y-auto no-scrollbar">
    <div class="flex items-center justify-between mb-8">
      <div><h2 class="text-2xl font-black flex items-center"><span class="material-symbols-outlined mr-3 text-primary text-3xl">account_balance_wallet</span>链上监听面板</h2><p class="text-sm text-slate-400 mt-1">管理 USDT 地址监听任务，支持 TRC20 与 BSC 网络实时扫描。</p></div>
      <div class="flex items-center space-x-3">
        <button @click="loadMonitors" class="p-3 bg-surface-container hover:bg-surface-container-high rounded-2xl transition-all"><span class="material-symbols-outlined text-primary">refresh</span></button>
        <button @click="showAddModal=true" class="px-5 py-3 bg-primary text-on-primary rounded-2xl font-black shadow-lg flex items-center space-x-2"><span class="material-symbols-outlined text-base">add</span><span>新增监听</span></button>
      </div>
    </div>
    <div class="grid grid-cols-1 md:grid-cols-4 gap-6 mb-6">
      <div class="bg-surface-container-lowest p-6 rounded-3xl border border-outline-variant/10 shadow-sm flex items-center space-x-4">
        <div class="w-12 h-12 rounded-2xl bg-primary/10 flex items-center justify-center text-primary"><span class="material-symbols-outlined">monitoring</span></div>
        <div><div class="text-[10px] font-black text-slate-400 uppercase tracking-widest">监听总数</div><div class="text-2xl font-black">{{ monitors.length }}</div></div>
      </div>
      <div v-for="pd in ['day','week','month']" :key="pd" class="bg-surface-container-lowest p-6 rounded-3xl border border-outline-variant/10 shadow-sm">
        <div class="text-[10px] font-black text-slate-400 uppercase tracking-widest mb-1">{{ pd==='day'?'当日盈亏':pd==='week'?'周盈亏':'月盈亏' }}</div>
        <div class="text-sm">入 {{ fmt(pnl[pd]?.inflow) }} / 出 {{ fmt(pnl[pd]?.outflow) }}</div>
        <div class="text-2xl font-black" :class="Number(pnl[pd]?.profit||0)>=0?'text-green-600':'text-red-600'">{{ fmt(pnl[pd]?.profit) }}</div>
      </div>
    </div>
    <div class="bg-surface-container-lowest rounded-3xl border border-outline-variant/10 shadow-sm overflow-hidden">
      <div v-if="isLoading" class="flex flex-col items-center justify-center py-20"><div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div></div>
      <div v-else-if="monitors.length===0" class="flex flex-col items-center justify-center py-20 text-slate-300 italic"><span class="material-symbols-outlined text-6xl mb-4 opacity-20">wallet_off</span><p>暂无链上监听任务</p></div>
      <div v-else class="overflow-x-auto"><table class="w-full text-left border-collapse">
        <thead><tr class="bg-surface-container-low border-b border-outline-variant/10">
          <th class="px-4 py-4 text-[10px] font-black uppercase text-slate-400">选择</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">监听ID</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">地址</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">网络</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">备注</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">近20笔盈亏</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400 text-right">操作</th>
        </tr></thead>
        <tbody class="divide-y divide-outline-variant/5">
          <tr v-for="m in monitors" :key="m.monitor_id" class="hover:bg-primary/5 transition-colors">
            <td class="px-4 py-4"><input type="checkbox" :checked="selectedIds.includes(m.monitor_id)" @change="toggleSelect(m.monitor_id)" class="rounded text-primary"/></td>
            <td class="px-6 py-4 text-xs font-bold">#{{ m.monitor_id }}</td>
            <td class="px-6 py-4 text-xs font-mono max-w-[300px] truncate">{{ m.address }}</td>
            <td class="px-6 py-4"><span class="px-2 py-1 bg-primary/10 text-primary text-[10px] font-black rounded-lg">{{ String(m.network||'').toUpperCase() }}</span></td>
            <td class="px-6 py-4 text-xs"><input :value="m.remark||''" @change="updateRemark(m,($event.target as HTMLInputElement).value)" class="bg-surface-container border-none rounded px-2 py-1 text-xs w-40" placeholder="可备注"/></td>
            <td class="px-6 py-4 text-xs">入 {{ fmt(m.inflow_20) }} / 出 {{ fmt(m.outflow_20) }}<div :class="Number(m.profit_20||0)>=0?'text-green-600 font-bold':'text-red-600 font-bold'">{{ fmt(m.profit_20) }}</div></td>
            <td class="px-6 py-4 text-right"><button @click="deleteMonitor(m.monitor_id)" class="px-3 py-2 bg-error/10 text-error text-[10px] font-black rounded-xl hover:bg-error/20">删除</button></td>
          </tr>
        </tbody>
      </table></div>
    </div>
    <transition name="fade"><div v-if="showAddModal" class="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/50 backdrop-blur-sm">
      <div class="bg-surface-container-lowest w-full max-w-md rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95">
        <div class="px-8 py-6 border-b flex items-center justify-between"><h3 class="text-lg font-black">新增链上监听</h3><button @click="showAddModal=false" class="w-10 h-10 flex items-center justify-center hover:bg-surface-container-high rounded-full"><span class="material-symbols-outlined">close</span></button></div>
        <div class="p-6 space-y-4">
          <input v-model="newMonitor.user_id" type="number" placeholder="用户 ID" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm"/>
          <input v-model="newMonitor.address" type="text" placeholder="USDT 地址" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm"/>
          <select v-model="newMonitor.network" class="w-full px-4 py-3 bg-surface-container rounded-xl border-none text-sm"><option value="trc20">TRC20</option><option value="bsc">BSC</option></select>
        </div>
        <div class="px-8 py-5 bg-surface-container-low flex justify-end space-x-3 border-t">
          <button @click="showAddModal=false" class="px-5 py-2.5 text-sm font-bold text-slate-500">取消</button>
          <button @click="addMonitor" class="px-6 py-2.5 bg-primary text-on-primary text-sm font-black rounded-xl">添加</button>
        </div>
      </div>
    </div></transition>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, watch } from 'vue'
import { useWsStore } from '@/stores/ws'
const wsStore = useWsStore()
const token = () => localStorage.getItem('admin_token') || ''
const authH = () => ({ 'Authorization': `Bearer ${token()}`, 'Content-Type': 'application/json' })
const monitors = ref<any[]>([])
const isLoading = ref(false)
const showAddModal = ref(false)
const selectedIds = ref<number[]>([])
const pnl = ref<Record<string,any>>({ day:{}, week:{}, month:{} })
const newMonitor = reactive({ user_id:'', address:'', network:'trc20' })
onMounted(() => { loadMonitors(); loadPnl() })
watch(() => wsStore.lastEvent, (evt) => {
  if (!evt) return
  if (evt.type === 'monitor_update') { loadMonitors(); loadPnl() }
})
async function loadMonitors() { isLoading.value=true; try { const res=await fetch('/api/monitors',{headers:authH()}); const d=await res.json(); if(d.status==='ok') monitors.value=d.data||[] } finally { isLoading.value=false } }
async function loadPnl() { try { const res=await fetch('/api/monitors/pnl_summary',{headers:authH()}); const d=await res.json(); if(d.status==='ok') pnl.value=d.data||pnl.value } catch {} }
function toggleSelect(id:number) { const i=selectedIds.value.indexOf(id); if(i>=0) selectedIds.value.splice(i,1); else selectedIds.value.push(id) }
async function addMonitor() { await fetch('/api/monitors',{method:'POST',headers:authH(),body:JSON.stringify(newMonitor)}); showAddModal.value=false; newMonitor.user_id=''; newMonitor.address=''; newMonitor.network='trc20'; await loadMonitors() }
async function deleteMonitor(id:number) { if(!confirm('确定删除？')) return; await fetch('/api/monitors',{method:'DELETE',headers:authH(),body:JSON.stringify({monitor_id:id})}); await loadMonitors() }
async function updateRemark(m:any,remark:string) { m.remark=remark; await fetch(`/api/monitors/${m.monitor_id}/remark`,{method:'POST',headers:authH(),body:JSON.stringify({remark})}) }
function fmt(v:any) { return Number(v||0).toFixed(2) }
</script>
