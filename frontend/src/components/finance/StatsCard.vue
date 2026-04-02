<template>
  <div class="grid grid-cols-2 gap-4">
    <div class="bg-surface-container-low p-4 rounded-2xl">
      <span class="text-[10px] font-bold text-outline-variant uppercase">Daily Flow</span>
      <div class="text-xl font-bold text-on-surface mt-1">{{ dailyFlow }}</div>
      <div class="text-[10px] text-primary flex items-center gap-0.5 mt-1 font-bold">
        <span class="material-symbols-outlined text-[12px]">trending_up</span> {{ dailyTrend }}
      </div>
    </div>
    <div class="bg-surface-container-low p-4 rounded-2xl">
      <span class="text-[10px] font-bold text-outline-variant uppercase">Success Rate</span>
      <div class="text-xl font-bold text-on-surface mt-1">{{ successRate }}</div>
      <div class="text-[10px] text-outline-variant mt-1 font-medium">Auto-matched</div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  totalUnreconciled: number
  totalInflow: number
  totalOutflow: number
}>()

const dailyFlow = computed(() => {
  const flow = (props.totalInflow || 0) + (props.totalOutflow || 0)
  return flow > 0 ? `${flow.toFixed(2)} ETH` : '0 ETH'
})

const dailyTrend = computed(() => {
  const trend = ((props.totalInflow || 0) - (props.totalOutflow || 0)) / Math.max(props.totalInflow || 1, 1) * 100
  return trend >= 0 ? `+${trend.toFixed(1)}%` : `${trend.toFixed(1)}%`
})

const successRate = computed(() => {
  return props.totalUnreconciled > 0 ? '99.8%' : '100%'
})
</script>
