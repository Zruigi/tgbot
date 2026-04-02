<template>
  <div class="bg-surface-container-lowest/90 backdrop-blur-md p-6 rounded-2xl shadow-xl border border-primary/5">
    <h2 class="text-lg font-bold text-on-surface mb-6 flex items-center gap-2">
      <span class="material-symbols-outlined text-primary">account_balance</span>
      对账操作面板
    </h2>
    
    <div class="space-y-4">
      <!-- Account Selector -->
      <div class="flex flex-col gap-2">
        <label class="text-xs font-bold text-outline uppercase tracking-wider">Accounting Account</label>
        <div class="relative">
          <select v-model="selectedAccount" class="w-full bg-surface-container-low border-none rounded-xl py-3 px-4 text-sm font-medium focus:ring-2 focus:ring-primary-container/40 appearance-none">
            <option value="">-- Select Account --</option>
            <option v-for="cat in categories" :key="cat.id" :value="cat.code">
              {{ cat.code }} - {{ cat.name }}
            </option>
          </select>
          <span class="material-symbols-outlined absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none text-on-surface-variant">expand_more</span>
        </div>
      </div>

      <!-- Amount Display -->
      <div class="flex flex-col gap-2">
        <label class="text-xs font-bold text-outline uppercase tracking-wider">Allocation Amount</label>
        <div class="relative">
          <input class="w-full bg-surface-container-low border-none rounded-xl py-4 px-4 text-xl font-bold text-on-surface focus:ring-2 focus:ring-primary-container/40" 
                 type="text" 
                 :value="amount"
                 readonly/>
          <span class="absolute right-4 top-1/2 -translate-y-1/2 font-bold text-primary">{{ tokenSymbol }}</span>
        </div>
        <p class="text-[10px] text-outline-variant px-1">Maximum available to allocate: {{ amount }} {{ tokenSymbol }}</p>
      </div>

      <!-- Confirm Button -->
      <div class="pt-6">
        <button @click="handleConfirm" 
                :disabled="!selectedAccount || isSubmitting"
                class="w-full py-4 bg-gradient-to-r from-primary to-primary-container text-white rounded-xl font-bold text-lg shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all disabled:opacity-50 disabled:cursor-not-allowed">
          {{ isSubmitting ? '处理中...' : 'Confirm Reconciliation' }}
        </button>
        <p class="text-center text-[11px] text-outline mt-4 flex items-center justify-center gap-1">
          <span class="material-symbols-outlined text-[14px]">verified</span>
          Blockchain audit trail will be generated
        </p>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { useFinanceStore } from '@/stores/finance'
import type { AccountCategory } from '@/stores/finance'

const props = defineProps<{
  ledgerId: number
  amount: number
  tokenSymbol: string
  categories: AccountCategory[]
}>()

const emit = defineEmits<{
  confirmed: []
  error: [message: string]
}>()

const financeStore = useFinanceStore()
const selectedAccount = ref('')
const isSubmitting = ref(false)

const handleConfirm = async () => {
  if (!selectedAccount.value) {
    emit('error', 'Please select an accounting account')
    return
  }

  isSubmitting.value = true
  try {
    const success = await financeStore.confirmReconciliation(
      props.ledgerId,
      selectedAccount.value,
      props.amount
    )
    if (success) {
      emit('confirmed')
      selectedAccount.value = ''
    } else {
      emit('error', financeStore.error || 'Reconciliation failed')
    }
  } finally {
    isSubmitting.value = false
  }
}
</script>
