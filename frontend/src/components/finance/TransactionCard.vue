<template>
  <div class="group bg-surface-container-lowest/90 backdrop-blur-md p-5 rounded-2xl shadow-sm border border-outline-variant/10 hover:shadow-md transition-all duration-300 hover:scale-[1.01] cursor-pointer"
       @click="$emit('select')">
    <div class="flex justify-between items-start mb-3">
      <div class="flex flex-col">
        <span class="text-[10px] text-outline uppercase font-bold tracking-widest">Transaction ID</span>
        <span class="text-sm font-mono font-semibold text-on-surface">{{ `TXN-${ledger.ledger_id}` }}</span>
      </div>
      <div class="bg-error-container/20 text-error text-[10px] font-bold px-2 py-1 rounded-md">
        {{ ledger.reconciled_status || 'Unprocessed' }}
      </div>
    </div>
    <div class="flex items-end justify-between">
      <div>
        <div class="text-2xl font-bold text-on-surface tracking-tight">{{ ledger.amount }} {{ ledger.token_symbol }}</div>
        <div class="text-xs text-outline-variant flex items-center gap-1 mt-1">
          <span class="material-symbols-outlined text-[14px]">schedule</span>
          {{ formatDate(ledger.created_at) }}
        </div>
      </div>
      <button class="bg-surface-container-low text-primary px-4 py-2 rounded-xl text-sm font-semibold flex items-center gap-2 hover:bg-primary hover:text-white transition-all active:scale-95">
        Select
        <span class="material-symbols-outlined text-sm">chevron_right</span>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import type { UnreconciledLedger } from '@/stores/finance'

defineProps<{
  ledger: UnreconciledLedger
}>()

defineEmits<{
  select: []
}>()

const formatDate = (dateStr: string) => {
  const date = new Date(dateStr)
  return date.toLocaleString('zh-CN', { 
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  })
}
</script>
